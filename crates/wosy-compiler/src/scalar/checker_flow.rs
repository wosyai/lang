use std::{borrow::Cow, collections::HashMap, rc::Rc};

use num_bigint::BigInt;

use super::{
    checker_directive::CheckerOperation,
    control::{AssignmentReplacement, BindingInitialization, BindingRegion, BlockItemKind, BlockPosition, CheckedCallSignature, ConditionalRegion, FinalOutputSlot},
    reference::{BlockOrdinal, DirectDestinationIdentity, FunctionCallSites, OccurrenceEdge, OccurrenceNode, OccurrenceOrdinal, OccurrenceParent, OccurrenceStructure, PlainNameIdentity},
    symbolic_integer::{IntegerIntersection, IntegerSet},
    typecheck::is_integer_type,
    BinaryOperator, ReferenceBindingId, ScalarExpression, ScalarType, UnaryOperator,
};
use super::{symbolic_integer::IntegerRelation, reference::CheckerIntegerCallArgument};

#[cfg(feature = "internal-checker")]
use super::symbolic_integer::IntegerShowConstraints;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum CheckerPredicate {
    Boolean(bool),
    #[cfg_attr(not(feature = "internal-checker"), allow(dead_code))]
    ProofReflexiveBranch(bool),
    Integer(IntegerPredicate),
    EntryDomain { binding: ReferenceBindingId, values: IntegerSet },
    DirectiveDisjunction { binding: ReferenceBindingId, ty: ScalarType, atoms: [(IntegerRelation, BigInt); 2] },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct IntegerPredicate {
    pub(super) binding: ReferenceBindingId,
    pub(super) ty: ScalarType,
    pub(super) operator: BinaryOperator,
    pub(super) bound: BigInt,
}

pub(super) struct IntegerComparison<'a> {
    pub(super) left: &'a ScalarExpression,
    pub(super) right: &'a ScalarExpression,
    pub(super) name: &'a str,
    pub(super) operator: &'a BinaryOperator,
    pub(super) bound: Cow<'a, BigInt>,
}

impl<'a> IntegerComparison<'a> {
    pub(super) fn from_expression(expression: &'a ScalarExpression) -> Option<Self> {
        let ScalarExpression::Binary { operator, left, right, .. } = expression else {
            return None;
        };
        if !matches!(
            operator,
            BinaryOperator::Equal | BinaryOperator::NotEqual | BinaryOperator::Less
                | BinaryOperator::LessEqual | BinaryOperator::Greater | BinaryOperator::GreaterEqual
        ) {
            return None;
        }
        let ScalarExpression::Name { name, .. } = left.as_ref() else {
            return None;
        };
        let bound = concrete_integer_literal(right)?;
        Some(Self { left, right, name, operator, bound })
    }

    pub(super) fn typed(
        self,
        left_type: &ScalarType,
        right_type: &ScalarType,
        binding: ReferenceBindingId,
    ) -> Option<IntegerPredicate> {
        if left_type != right_type || !is_integer_type(left_type) {
            return None;
        }
        Some(IntegerPredicate {
            binding,
            ty: left_type.clone(),
            operator: self.operator.clone(),
            bound: self.bound.into_owned(),
        })
    }
}

pub(super) fn concrete_integer_literal(expression: &ScalarExpression) -> Option<Cow<'_, BigInt>> {
    match expression {
        ScalarExpression::Integer { value, .. } => Some(Cow::Borrowed(value)),
        ScalarExpression::Unary { operator: UnaryOperator::Negate, operand, .. } => {
            let ScalarExpression::Integer { value, .. } = operand.as_ref() else {
                return None;
            };
            Some(Cow::Owned(-value))
        }
        _ => None,
    }
}

pub(super) struct ImportedLocal<'a> {
    pub(super) position: usize,
    pub(super) values: ImportedIntegerValues<'a>,
    pub(super) guards: Vec<ImportedEntryGuard<'a>>,
}

pub(super) enum ImportedIntegerValues<'a> {
    Ready(IntegerSet, std::marker::PhantomData<&'a ()>),
    #[cfg(feature = "internal-checker")]
    Deferred {
        slot: usize,
        domain: &'a IntegerSet,
        issue: &'a mut dyn FnMut() -> Option<IntegerSet>,
    },
}

impl ImportedIntegerValues<'_> {
    pub(super) fn ready(values: IntegerSet) -> Self {
        Self::Ready(values, std::marker::PhantomData)
    }
}

pub(super) struct ImportedEntryGuard<'a> {
    pub(super) slot: usize,
    pub(super) relation: IntegerRelation,
    pub(super) bound: &'a BigInt,
}

trait ScopedAuthoredReturn<'a>: Copy {
    const ENABLED: bool;
    fn at(
        self, structure: &OccurrenceStructure, block: BlockOrdinal, slot: FinalOutputSlot, ty: &ScalarType,
    ) -> Option<&'a BigInt>;
}

impl<'a> ScopedAuthoredReturn<'a> for () {
    const ENABLED: bool = false;

    fn at(self, _: &OccurrenceStructure, _: BlockOrdinal, _: FinalOutputSlot, _: &ScalarType) -> Option<&'a BigInt> {
        None
    }
}

#[cfg(feature = "internal-checker")]
#[derive(Clone, Copy)]
struct NamedAuthoredReturn<'a> {
    body: &'a super::ScalarBlock,
    value: &'a std::cell::OnceCell<Option<BigInt>>,
}

#[cfg(feature = "internal-checker")]
impl<'a> ScopedAuthoredReturn<'a> for NamedAuthoredReturn<'a> {
    const ENABLED: bool = true;

    fn at(
        self, structure: &OccurrenceStructure, block: BlockOrdinal, slot: FinalOutputSlot,
        ty: &ScalarType,
    ) -> Option<&'a BigInt> {
        if block != structure.root || slot.output != 0 || structure.blocks.get(block.0)?.span != self.body.span
            || self.body.items.len().checked_sub(1)? != slot.item
        {
            return None;
        }
        let [output] = self.body.final_output_values.as_slice() else { return None; };
        if output.position != slot.output || output.span != slot.span
            || structure.edges.get(&(OccurrenceParent::Block(block), OccurrenceEdge::FinalOutput {
                item: slot.item, output: slot.output,
            })) != Some(&OccurrenceNode::Expression(slot.expression))
        {
            return None;
        }
        self.value.get_or_init(|| {
            let ScalarExpression::Binary { operator, left, right, .. } = &output.value else { return None; };
            if !matches!(operator, BinaryOperator::Add | BinaryOperator::Subtract) {
                return None;
            }
            let parent = OccurrenceParent::Expression(slot.expression);
            let typed_operand = |edge| {
                let Some(OccurrenceNode::Expression(ordinal)) = structure.edges.get(&(parent, edge)) else { return false; };
                structure.checked_types.expressions.get(ordinal) == Some(ty)
            };
            if structure.checked_types.expressions.get(&slot.expression) != Some(ty)
                || !typed_operand(OccurrenceEdge::BinaryLeft) || !typed_operand(OccurrenceEdge::BinaryRight)
            {
                return None;
            }
            let left = concrete_integer_literal(left)?;
            let right = concrete_integer_literal(right)?;
            let (min, max) = super::typecheck::integer_bounds(ty)?;
            let fits = |value: &BigInt| value >= &min && value <= &max;
            if !fits(&left) || !fits(&right) {
                return None;
            }
            #[cfg(all(test, feature = "internal-checker"))]
            AUTHORED_RETURN_CALCULATIONS.with(|count| count.set(count.get() + 1));
            let value = if *operator == BinaryOperator::Add { left.as_ref() + right.as_ref() }
                else { left.as_ref() - right.as_ref() };
            fits(&value).then_some(value)
        }).as_ref()
    }
}

#[cfg(all(test, feature = "internal-checker"))]
thread_local! {
    pub(super) static AUTHORED_RETURN_CALCULATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

type IntegerRow = Vec<Rc<IntegerSet>>;
#[derive(Default)]
struct IntegerRoots<'a> {
    axes: Vec<(&'a ReferenceBindingId, &'a ScalarType)>,
    by_binding: HashMap<&'a ReferenceBindingId, usize>,
    parameters: usize,
    reached_obligation: bool,
    proof_reflexive_branches: bool,
    imported_local: Option<ImportedLocal<'a>>,
}

impl<'a> IntegerRoots<'a> {
    fn add(&mut self, binding: &'a ReferenceBindingId, ty: &'a ScalarType) {
        let previous = self.by_binding.insert(binding, self.axes.len());
        assert!(previous.is_none());
        self.axes.push((binding, ty));
    }

    fn lookup(&self, binding: &ReferenceBindingId) -> Option<(usize, &'a ScalarType)> {
        let index = *self.by_binding.get(binding)?;
        Some((index, self.axes[index].1))
    }

    fn truncate(&mut self, depth: usize) {
        for (binding, _) in self.axes.drain(depth..) {
            self.by_binding.remove(binding);
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct IntegerInputs(Vec<IntegerRow>);

impl IntegerInputs {
    pub(super) fn closed() -> Self {
        Self(vec![Vec::new()])
    }

    fn singleton_at(&self, axis: usize) -> Option<&BigInt> {
        let (first, rest) = self.0.split_first()?;
        let value = first[axis].singleton_value()?;
        rest.iter().all(|row| row[axis].singleton_value() == Some(value)).then_some(value)
    }

    fn project_axis(&self, axis: usize) -> Option<Rc<IntegerSet>> {
        let (first, rest) = self.0.split_first()?;
        let first = &first[axis];
        Some(if rest.iter().all(|row| Rc::ptr_eq(first, &row[axis])) {
            Rc::clone(first)
        } else {
            Rc::new(IntegerSet::union(self.0.iter().map(|row| row[axis].as_ref())))
        })
    }

    fn restrict_axis(&mut self, axis: usize, survivors: &IntegerSet) {
        self.0.retain_mut(|row| match row[axis].intersection(survivors) {
            IntegerIntersection::Unchanged => true,
            IntegerIntersection::Empty => false,
            IntegerIntersection::Changed(retained) => {
                row[axis] = Rc::new(retained);
                true
            }
        });
    }

    fn intersection_witness(&self, axis: usize, values: &IntegerSet) -> Self {
        let mut witness = Self::default();
        for row in &self.0 {
            match row[axis].intersection(values) {
                IntegerIntersection::Empty => {}
                IntegerIntersection::Unchanged => witness.0.push(row.clone()),
                IntegerIntersection::Changed(intersection) => {
                    let mut row = row.clone();
                    row[axis] = Rc::new(intersection);
                    witness.0.push(row);
                }
            }
        }
        witness
    }

    fn restrict_axis_or_witness(&mut self, axis: usize, values: &IntegerSet) -> Self {
        let mut rejected_prefix = 0;
        let mut surviving = false;
        self.0.retain_mut(|row| match row[axis].intersection(values) {
            IntegerIntersection::Empty if !surviving => {
                rejected_prefix += 1;
                true
            }
            IntegerIntersection::Empty => false,
            IntegerIntersection::Unchanged => {
                surviving = true;
                true
            }
            IntegerIntersection::Changed(intersection) => {
                surviving = true;
                row[axis] = Rc::new(intersection);
                true
            }
        });
        if !surviving {
            return std::mem::take(self);
        }
        drop(self.0.drain(..rejected_prefix));
        Self::default()
    }

    fn split(
        self,
        roots: &IntegerRoots<'_>,
        predicate: &CheckerPredicate,
    ) -> Option<(Self, Self, Option<usize>)> {
        let predicate = match predicate {
            CheckerPredicate::Boolean(true) => return Some((self, Self::default(), None)),
            CheckerPredicate::Boolean(false) => return Some((Self::default(), self, None)),
            CheckerPredicate::ProofReflexiveBranch(value) if roots.proof_reflexive_branches => {
                return Some(if *value { (self, Self::default(), None) } else { (Self::default(), self, None) });
            }
            CheckerPredicate::Integer(predicate) => predicate,
            CheckerPredicate::EntryDomain { .. } | CheckerPredicate::DirectiveDisjunction { .. } | CheckerPredicate::ProofReflexiveBranch(_) => return None,
        };
        let (index, ty) = roots.lookup(&predicate.binding)?;
        if ty != &predicate.ty {
            return None;
        }
        let (passing, failing) = self.split_axis(index, predicate)?;
        Some((passing, failing, Some(index)))
    }

    fn split_axis(self, index: usize, predicate: &IntegerPredicate) -> Option<(Self, Self)> {
        let mut passing = Self::default();
        let mut failing = Self::default();
        for mut row in self.0 {
            let (yes, no) = row[index].split(&predicate.operator, &predicate.bound)?;
            if yes.is_empty() {
                failing.0.push(row);
            } else if no.is_empty() {
                passing.0.push(row);
            } else {
                let mut alternate = row.clone();
                row[index] = Rc::new(yes);
                alternate[index] = Rc::new(no);
                passing.0.push(row);
                failing.0.push(alternate);
            }
        }
        Some((passing, failing))
    }

    fn failing_witness(&self, roots: &IntegerRoots<'_>, predicate: &CheckerPredicate) -> Option<Self> {
        let predicate = match predicate {
            CheckerPredicate::Boolean(true) => return Some(Self::default()),
            CheckerPredicate::Boolean(false) => return Some(Self(self.0.clone())),
            CheckerPredicate::Integer(predicate) => predicate,
            CheckerPredicate::EntryDomain { .. } | CheckerPredicate::DirectiveDisjunction { .. } | CheckerPredicate::ProofReflexiveBranch(_) => return None,
        };
        let (index, ty) = roots.lookup(&predicate.binding)?;
        if ty != &predicate.ty {
            return None;
        }
        let relation = IntegerRelation::from_operator(&predicate.operator)?.complement();
        let mut failing = Self::default();
        for row in &self.0 {
            let axis = &row[index];
            let rejected = axis.keep_relation(relation, &predicate.bound);
            if !rejected.is_empty() {
                let mut row = row.clone();
                if rejected != **axis {
                    row[index] = Rc::new(rejected);
                }
                failing.0.push(row);
            }
        }
        Some(failing)
    }

    fn join(mut self, mut other: Self, axis: Option<usize>) -> Self {
        if self.is_unreachable() {
            return other;
        }
        if other.is_unreachable() {
            return self;
        }
        let axis = axis.expect("both nonempty paths require a partition axis");
        if self.0.len() == 1 && other.0.len() == 1 {
            let left = &mut self.0[0];
            let right = &other.0[0];
            if left.iter().zip(right).enumerate().all(|(index, (a, b))| {
                index == axis || Rc::ptr_eq(a, b)
            }) {
                let right = other.0.pop().unwrap();
                merge_axis(left, right, axis);
            } else {
                self.0.append(&mut other.0);
            }
            return self;
        }

        Self::compact_axis(self.0.into_iter().chain(other.0), axis)
    }

    fn overwrite(mut self, axis: usize, value: &BigInt) -> Self {
        let value = Rc::new(IntegerSet::singleton(value));
        for row in &mut self.0 {
            row[axis] = Rc::clone(&value);
        }
        if self.0.len() < 2 {
            return self;
        }
        Self::compact_axis(self.0, axis)
    }

    fn append_local(&mut self, values: IntegerSet) {
        let value = Rc::new(values);
        for row in &mut self.0 {
            row.push(Rc::clone(&value));
        }
    }

    fn drop_locals(&mut self, depth: usize) {
        for row in &mut self.0 {
            row.truncate(depth);
        }
    }

    fn compact_axis(rows: impl IntoIterator<Item = IntegerRow>, axis: usize) -> Self {
        let mut joined = Self::default();
        let mut representatives = HashMap::new();
        for row in rows {
            let key: Vec<_> = row[..axis].iter().chain(&row[axis + 1..]).map(Rc::as_ptr).collect();
            match representatives.entry(key) {
                std::collections::hash_map::Entry::Occupied(entry) => {
                    merge_axis(&mut joined.0[*entry.get()], row, axis);
                }
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(joined.0.len());
                    joined.0.push(row);
                }
            }
        }
        joined
    }

    fn is_unreachable(&self) -> bool {
        self.0.is_empty()
    }

    #[cfg(any(test, feature = "internal-checker"))]
    pub(super) fn row_count(&self) -> usize {
        self.0.len()
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn intervals_at(
        &self,
        row: usize,
        axis: usize,
    ) -> Option<impl ExactSizeIterator<Item = (&BigInt, &BigInt)>> {
        Some(self.0.get(row)?.get(axis)?.intervals())
    }

    #[cfg(test)]
    pub(super) fn includes(&self, coordinates: &[u128]) -> bool {
        self.includes_coordinates(coordinates)
    }

    #[cfg(test)]
    pub(super) fn includes_signed(&self, coordinates: &[i128]) -> bool {
        self.includes_coordinates(coordinates)
    }

    #[cfg(test)]
    fn includes_coordinates<T: Copy + Into<BigInt>>(&self, coordinates: &[T]) -> bool {
        self.0.iter().any(|row| {
            row.len() == coordinates.len()
                && row.iter().zip(coordinates).all(|(axis, value)| {
                    axis.split(&BinaryOperator::Equal, &(*value).into())
                        .is_some_and(|(matching, _)| !matching.is_empty())
                })
        })
    }
}

fn merge_axis(representative: &mut IntegerRow, row: IntegerRow, axis: usize) {
    if Rc::ptr_eq(&representative[axis], &row[axis]) {
        return;
    }
    let replacement = row.into_iter().nth(axis).expect("canonical integer axis");
    let previous = std::mem::replace(&mut representative[axis], replacement);
    let target = Rc::make_mut(&mut representative[axis]);
    *target = Rc::unwrap_or_clone(previous).join(std::mem::take(target));
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct IntegerCheckFailure<'a> {
    pub(super) directive: OccurrenceOrdinal,
    pub(super) state: IntegerInputs,
    pub(super) axes: Vec<(&'a ReferenceBindingId, &'a ScalarType)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum IntegerReturn<'a> {
    Alias(&'a ReferenceBindingId),
    Const(&'a BigInt),
}

#[cfg(feature = "internal-checker")]
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct ImmutableEntryRange<'a> {
    pub(super) binding: &'a ReferenceBindingId,
    pub(super) lower: &'a BigInt,
    pub(super) upper: &'a BigInt,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct IntegerReturningPath<'a> {
    pub(super) value: IntegerReturn<'a>,
    pub(super) ty: &'a ScalarType,
    pub(super) state: IntegerInputs,
    pub(super) axes: Vec<&'a ReferenceBindingId>,
}

pub(super) enum IntegerShowProof<'a> {
    Equality(IntegerReturn<'a>),
    Sequence(Vec<IntegerReturn<'a>>),
    #[cfg(feature = "internal-checker")]
    MixedSequence {
        formals: &'a [ReferenceBindingId],
        output: IntegerShowConstraints<'a>,
        entry: [Option<IntegerRelation>; 2],
    },
    #[cfg(feature = "internal-checker")]
    Output(IntegerShowOutput),
    #[cfg(feature = "internal-checker")]
    EntryComparison(ImmutableProofOutputComparison<'a>),
    #[cfg(feature = "internal-checker")]
    GuardedOutput {
        guard: IntegerPredicate,
        claims: GuardedOutputClaims,
    },
    Conditional {
        guard: CheckerPredicate,
        then_claim: IntegerReturn<'a>,
        else_claim: IntegerReturn<'a>,
    },
}

#[cfg(feature = "internal-checker")]
pub(super) struct ActualFlatShow<'a> {
    bound: &'a BigInt,
    relation: IntegerRelation,
    extra: ActualFlatClaim<'a>,
}

#[cfg(feature = "internal-checker")]
enum ActualFlatClaim<'a> {
    None,
    Equal(&'a BigInt),
    Impossible,
    Output(IntegerShowOutput, Option<usize>),
    Constraints(IntegerShowConstraints<'a>),
    Entry { slot: usize, relation: IntegerRelation },
    MixedEntry(IntegerShowConstraints<'a>, ActualEntryPredicates),
    Guarded {
        slot: usize,
        relation: IntegerRelation,
        bound: BigInt,
        claims: GuardedOutputClaims,
    },
}

#[cfg(feature = "internal-checker")]
enum ActualEntryPredicates {
    One { slot: usize, relation: IntegerRelation },
    Both(IntegerRelation, IntegerRelation),
}

#[cfg(feature = "internal-checker")]
impl ActualEntryPredicates {
    fn holds(&self, value: &BigInt, arguments: &[&BigInt]) -> Option<bool> {
        match self {
            Self::One { slot, relation } => Some(relation.accepts(value.cmp(arguments.get(*slot)?))),
            Self::Both(first, second) => {
                let [a, b] = arguments else { return None; };
                Some(first.accepts(value.cmp(a)) && second.accepts(value.cmp(b)))
            }
        }
    }
}

#[cfg(feature = "internal-checker")]
impl<'a> ActualFlatShow<'a> {
    pub(super) fn admit(
        proof: IntegerShowProof<'a>, mut relation: IntegerRelation, bound: &'a BigInt,
        bindings: &[ReferenceBindingId], arity: usize,
    ) -> Option<Self> {
        if bindings.len() != arity {
            return None;
        }
        let extra = match proof {
            IntegerShowProof::Equality(IntegerReturn::Const(value)) if value == bound => {
                relation = relation.intersection(IntegerRelation::from_operator(&BinaryOperator::Equal)?);
                ActualFlatClaim::None
            }
            IntegerShowProof::Equality(IntegerReturn::Const(value)) => ActualFlatClaim::Equal(value),
            IntegerShowProof::Sequence(claims) => {
                let mut expected = None;
                let mut impossible = false;
                for claim in claims {
                    let IntegerReturn::Const(value) = claim else { return None; };
                    if let Some(first) = expected {
                        impossible |= first != value;
                    } else {
                        expected = Some(value);
                    }
                }
                if impossible {
                    ActualFlatClaim::Impossible
                } else if let Some(value) = expected {
                    if value == bound {
                        relation = relation.intersection(IntegerRelation::from_operator(&BinaryOperator::Equal)?);
                        ActualFlatClaim::None
                    } else {
                        ActualFlatClaim::Equal(value)
                    }
                } else {
                    ActualFlatClaim::None
                }
            }
            IntegerShowProof::Output(IntegerShowOutput::Relation { relation: show, bound: show_bound }) if &show_bound == bound => {
                relation = relation.intersection(show);
                ActualFlatClaim::None
            }
            IntegerShowProof::Output(output) => {
                let skip = match &output {
                    IntegerShowOutput::Conjunction(atoms) => atoms.binary_search_by(|(_, literal)| literal.cmp(bound)).ok(),
                    _ => None,
                };
                if let (Some(index), IntegerShowOutput::Conjunction(atoms)) = (skip, &output) {
                    relation = relation.intersection(atoms[index].0);
                }
                ActualFlatClaim::Output(output, skip)
            }
            IntegerShowProof::MixedSequence { output, entry: [None, None], .. } => ActualFlatClaim::Constraints(output),
            IntegerShowProof::EntryComparison(entry) => {
                let slot = bindings.iter().position(|binding| binding == entry.binding)?;
                ActualFlatClaim::Entry { slot, relation: entry.relation }
            }
            IntegerShowProof::MixedSequence { formals, output, entry } if formals == bindings => {
                let checked = match entry {
                    [Some(first), None] if arity >= 1 => ActualEntryPredicates::One { slot: 0, relation: first },
                    [None, Some(second)] if arity == 2 => ActualEntryPredicates::One { slot: 1, relation: second },
                    [Some(first), Some(second)] if arity == 2 => ActualEntryPredicates::Both(first, second),
                    _ => return None,
                };
                ActualFlatClaim::MixedEntry(output, checked)
            }
            IntegerShowProof::GuardedOutput { guard, claims } => {
                let slot = bindings.iter().position(|binding| *binding == guard.binding)?;
                let relation = IntegerRelation::from_operator(&guard.operator)?;
                ActualFlatClaim::Guarded { slot, relation, bound: guard.bound, claims }
            }
            _ => return None,
        };
        Some(Self { bound, relation, extra })
    }

    pub(super) fn holds(&self, value: &BigInt, arguments: &[&BigInt]) -> Option<bool> {
        match &self.extra {
            ActualFlatClaim::Constraints(output) => return Some(output.holds_value(value)),
            ActualFlatClaim::Impossible => return Some(false),
            ActualFlatClaim::MixedEntry(output, entry) => {
                if !output.holds_value(value) {
                    return Some(false);
                }
                return entry.holds(value, arguments);
            }
            ActualFlatClaim::Entry { slot, relation } => {
                let entry = arguments.get(*slot)?;
                return Some(self.relation.accepts(value.cmp(self.bound)) && relation.accepts(value.cmp(entry)));
            }
            ActualFlatClaim::Guarded { slot, relation, bound, claims } => {
                let entry = arguments.get(*slot)?;
                if !self.relation.accepts(value.cmp(self.bound)) {
                    return Some(false);
                }
                let condition = relation.accepts((*entry).cmp(bound));
                return Some(match claims {
                    GuardedOutputClaims::Literal { yes, no } => {
                        let (relation, bound) = if condition { yes } else { no };
                        relation.accepts(value.cmp(bound))
                    }
                    GuardedOutputClaims::ImmutableEntry { yes, no } => {
                        let relation = if condition { yes } else { no };
                        relation.accepts(value.cmp(entry))
                    }
                });
            }
            _ => {}
        }
        Some(self.relation.accepts(value.cmp(self.bound)) && match &self.extra {
            ActualFlatClaim::None => true,
            ActualFlatClaim::Equal(expected) => value == *expected,
            ActualFlatClaim::Output(show, skip) => show.holds_value(value, *skip),
            _ => unreachable!(),
        })
    }
}

#[cfg(feature = "internal-checker")]
pub(super) fn holds_actual_mixed_entry(
    output: &IntegerShowConstraints<'_>, entry: &[Option<IntegerRelation>; 2],
    value: &BigInt, arguments: &[&BigInt],
) -> Option<bool> {
    if entry.iter().skip(arguments.len()).any(Option::is_some) {
        return None;
    }
    Some(output.holds_value(value) && arguments.iter().zip(entry).all(|(argument, relation)|
        relation.is_none_or(|relation| relation.accepts(value.cmp(*argument)))))
}

#[cfg(feature = "internal-checker")]
pub(super) enum GuardedOutputClaims {
    Literal { yes: (IntegerRelation, BigInt), no: (IntegerRelation, BigInt) },
    ImmutableEntry { yes: IntegerRelation, no: IntegerRelation },
}

#[cfg(feature = "internal-checker")]
pub(super) enum IntegerShowOutput {
    Relation { relation: IntegerRelation, bound: BigInt },
    Disjunction([(IntegerRelation, BigInt); 2]),
    Conjunction(Vec<(IntegerRelation, BigInt)>),
    Alternatives {
        atoms: Vec<(IntegerRelation, BigInt)>,
        clause_ends: Vec<usize>,
    },
}

#[cfg(feature = "internal-checker")]
impl IntegerShowOutput {
    fn clauses<'a>(
        atoms: &'a [(IntegerRelation, BigInt)], clause_ends: &'a [usize],
    ) -> impl Iterator<Item = &'a [(IntegerRelation, BigInt)]> {
        let mut start = 0;
        clause_ends.iter().map(move |&end| {
            let clause = &atoms[start..end];
            start = end;
            clause
        })
    }

    fn holds_value(&self, value: &BigInt, skip_conjunct: Option<usize>) -> bool {
        match self {
            Self::Relation { relation, bound } => relation.accepts(value.cmp(bound)),
            Self::Conjunction(atoms) => atoms.iter().enumerate().all(|(index, (relation, bound))|
                skip_conjunct == Some(index) || relation.accepts(value.cmp(bound))),
            Self::Disjunction([(left_relation, left), (right_relation, right)]) =>
                left_relation.accepts(value.cmp(left)) || right_relation.accepts(value.cmp(right)),
            Self::Alternatives { atoms, clause_ends } => Self::clauses(atoms, clause_ends).any(|clause|
                clause.iter().all(|(relation, bound)| relation.accepts(value.cmp(bound)))),
        }
    }

    fn normalize_union(&self, ty: &ScalarType) -> Option<IntegerSet> {
        match self {
            Self::Disjunction([(left_relation, left), (right_relation, right)]) =>
                IntegerSet::comparison_union(ty, (*left_relation, left), (*right_relation, right)),
            Self::Alternatives { atoms, clause_ends } =>
                IntegerSet::comparison_alternatives(ty, Self::clauses(atoms, clause_ends)),
            _ => None,
        }
    }
}

#[cfg(feature = "internal-checker")]
struct ForbiddenCutPair<'a> {
    lower: &'a BigInt,
    upper: &'a BigInt,
    mask: [bool; 5],
}

#[cfg(feature = "internal-checker")]
impl<'a> ForbiddenCutPair<'a> {
    fn new(first: (IntegerRelation, &'a BigInt), second: (IntegerRelation, &'a BigInt)) -> Self {
        use std::cmp::Ordering::{Equal, Greater, Less};
        let ((low, lower), (high, upper)) = if first.1 <= second.1 { (first, second) } else { (second, first) };
        let accepts = |a, b| low.accepts(a) && high.accepts(b);
        let mask = if lower == upper {
            [accepts(Less, Less), accepts(Equal, Equal), false, false, accepts(Greater, Greater)]
        } else {
            [accepts(Less, Less), accepts(Equal, Less),
             accepts(Greater, Less) && !consecutive_integer_cuts(lower, upper),
             accepts(Greater, Equal), accepts(Greater, Greater)]
        };
        Self { lower, upper, mask }
    }

    fn hits(&self, first: &BigInt, last: &BigInt) -> bool {
        let contains = |cut| first <= cut && cut <= last;
        (self.mask[0] && first < self.lower)
            || (self.mask[1] && contains(self.lower))
            || (self.mask[2] && first < self.upper && last > self.lower)
            || (self.mask[3] && contains(self.upper))
            || (self.mask[4] && last > self.upper)
    }
}

#[cfg(feature = "internal-checker")]
pub(super) fn integer_predicate_implies(
    first: (IntegerRelation, &BigInt), conclusion: (IntegerRelation, &BigInt),
    bounds: (&BigInt, &BigInt),
) -> bool {
    !ForbiddenCutPair::new(first, (conclusion.0.complement(), conclusion.1))
        .hits(bounds.0, bounds.1)
}

#[cfg(feature = "internal-checker")]
fn forbidden_guarded_cuts<'a>(
    guard: (IntegerRelation, &'a BigInt), otherwise: (IntegerRelation, &'a BigInt),
    yes: (IntegerRelation, &'a BigInt), no: (IntegerRelation, &'a BigInt),
) -> (ForbiddenCutPair<'a>, ForbiddenCutPair<'a>) {
    (
        ForbiddenCutPair::new(guard, (yes.0.complement(), yes.1)),
        ForbiddenCutPair::new(otherwise, (no.0.complement(), no.1)),
    )
}

#[cfg(feature = "internal-checker")]
fn consecutive_integer_cuts(lower: &BigInt, upper: &BigInt) -> bool {
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

fn binding_satisfies(
    state: &IntegerInputs,
    axes: &[&ReferenceBindingId],
    binding: &ReferenceBindingId,
    relation: IntegerRelation,
    bound: &BigInt,
) -> Option<bool> {
    let index = axes.iter().position(|known| *known == binding)?;
    Some(state.0.iter().all(|row| row[index].every(relation, bound)))
}

#[cfg(feature = "internal-checker")]
#[derive(Clone, Copy)]
enum CartesianRelation {
    Never,
    Equal,
    Disjoint,
    Always,
    Order { upper_against_lower: bool, relation: IntegerRelation },
}

#[cfg(feature = "internal-checker")]
impl CartesianRelation {
    fn prepare(relation: IntegerRelation) -> Self {
        use std::cmp::Ordering::{Equal, Greater, Less};
        match (relation.accepts(Less), relation.accepts(Greater), relation.accepts(Equal)) {
            (false, false, false) => Self::Never,
            (false, false, true) => Self::Equal,
            (true, true, false) => Self::Disjoint,
            (true, true, true) => Self::Always,
            (upper_against_lower, _, _) => Self::Order { upper_against_lower, relation },
        }
    }

    fn holds(self, result: &IntegerSet, old: &IntegerSet) -> Option<bool> {
        Some(match self {
            Self::Never => false,
            Self::Equal => result.singleton_value().is_some_and(|value| Some(value) == old.singleton_value()),
            Self::Disjoint => result.is_disjoint(old),
            Self::Always => true,
            Self::Order { upper_against_lower, relation } => {
                let (result_min, result_max) = result.extremes()?;
                let (old_min, old_max) = old.extremes()?;
                let worst = if upper_against_lower { result_max.cmp(old_min) } else { result_min.cmp(old_max) };
                relation.accepts(worst)
            }
        })
    }
}

#[cfg(feature = "internal-checker")]
pub(super) struct ImmutableProofOutputComparison<'a> {
    binding: &'a ReferenceBindingId,
    relation: IntegerRelation,
    other: Option<(&'a ReferenceBindingId, CartesianRelation)>,
}

#[cfg(feature = "internal-checker")]
impl<'a> ImmutableProofOutputComparison<'a> {
    pub(super) fn prepare(
        body: &'a FunctionCallSites, slot: usize, ty: &ScalarType, relation: IntegerRelation,
    ) -> Option<Self> {
        let bindings = body.boundary.checker_parameter_bindings.as_deref()?;
        let parameters = body.boundary.parameters.as_slice();
        let binding = bindings.get(slot)?;
        if &parameters.get(slot)?.ty != ty {
            return None;
        }
        let other = match (bindings, parameters) {
            ([_, other], [_, parameter]) if slot == 0 && &parameter.ty == ty => Some(other),
            ([other, _], [parameter, _]) if slot == 1 && &parameter.ty == ty => Some(other),
            _ => None,
        }.map(|other| (other, CartesianRelation::prepare(relation)));
        Some(Self { binding, relation, other })
    }

    pub(super) fn holds_existential_witness(
        &self, paths: &[IntegerReturningPath<'_>], domain: &IntegerSet,
    ) -> Option<bool> {
        for path in paths {
            let witness = path.axes.iter().position(|known| *known == self.binding)?;
            let other = match path.value {
                IntegerReturn::Alias(binding) if binding != self.binding => {
                    let (other, _) = self.other?;
                    if binding != other {
                        return None;
                    }
                    Some(path.axes.iter().position(|known| *known == other)?)
                }
                _ => None,
            };
            for row in &path.state.0 {
                let candidate = &row[witness];
                let matches = match (path.value, other) {
                    (IntegerReturn::Const(value), _) => candidate.singleton_value() == Some(value),
                    (IntegerReturn::Alias(_), Some(other)) => CartesianRelation::Equal.holds(&row[other], candidate)?,
                    (IntegerReturn::Alias(_), None) => true,
                };
                if !matches || !candidate.is_subset_of(domain) {
                    return Some(false);
                }
            }
        }
        Some(true)
    }

    pub(super) fn holds(
        &self, paths: &[IntegerReturningPath<'_>], output: Option<(IntegerRelation, &BigInt)>,
    ) -> Option<bool> {
        let mut proved = true;
        for path in paths {
            match path.value {
                IntegerReturn::Alias(alias) if alias != self.binding => {
                    let (other, comparison) = self.other?;
                    if alias != other {
                        return None;
                    }
                    if !proved {
                        continue;
                    }
                    let returned = path.axes.iter().position(|known| *known == alias)?;
                    let old = path.axes.iter().position(|known| *known == self.binding)?;
                    for row in &path.state.0 {
                        let result = &row[returned];
                        if !output.is_none_or(|(relation, bound)| result.every_at_extremes(relation, bound))
                            || !comparison.holds(result, &row[old])?
                        {
                            proved = false;
                            break;
                        }
                    }
                }
                IntegerReturn::Alias(_) if proved => {
                    proved = self.relation.accepts(std::cmp::Ordering::Equal) && match output {
                        Some((relation, bound)) => path.output_satisfies(relation, bound)?,
                        None => true,
                    };
                }
                IntegerReturn::Const(value) if proved => {
                    proved = output.is_none_or(|(relation, bound)| relation.accepts(value.cmp(bound)))
                        && path.binding_satisfies(self.binding, self.relation.reversed(), value)?;
                }
                _ => {}
            }
        }
        Some(proved)
    }
}

fn shows_return_equality(
    state: &IntegerInputs,
    axes: &[&ReferenceBindingId],
    value: &IntegerReturn<'_>,
    claim: &IntegerReturn<'_>,
) -> Option<bool> {
    if state.is_unreachable() {
        return Some(true);
    }
    match (value, claim) {
        (IntegerReturn::Const(actual), IntegerReturn::Const(expected)) => Some(actual == expected),
        (IntegerReturn::Alias(actual), IntegerReturn::Alias(expected)) => {
            (actual == expected).then_some(true)
        }
        (IntegerReturn::Const(value), IntegerReturn::Alias(binding))
        | (IntegerReturn::Alias(binding), IntegerReturn::Const(value)) => binding_satisfies(
            state,
            axes,
            binding,
            IntegerRelation::from_operator(&BinaryOperator::Equal)?,
            value,
        ),
    }
}

impl IntegerReturningPath<'_> {
    pub(super) fn binding_satisfies(
        &self,
        binding: &ReferenceBindingId,
        relation: IntegerRelation,
        bound: &BigInt,
    ) -> Option<bool> {
        binding_satisfies(&self.state, &self.axes, binding, relation, bound)
    }

    pub(super) fn output_satisfies(&self, relation: IntegerRelation, bound: &BigInt) -> Option<bool> {
        match self.value {
            IntegerReturn::Alias(binding) => self.binding_satisfies(binding, relation, bound),
            IntegerReturn::Const(value) => Some(relation.accepts(value.cmp(bound))),
        }
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn covers_immutable_entry_domain(
        &self, first: ImmutableEntryRange<'_>, second: Option<ImmutableEntryRange<'_>>,
    ) -> bool {
        if self.axes.first() != Some(&first.binding) {
            return false;
        }
        match second {
            None => self.state.0.iter().any(|row| row[0].covers_closed(first.lower, first.upper)),
            Some(second) if self.axes.get(1) == Some(&second.binding) => self.state.0.iter().any(|row| {
                row[0].covers_closed(first.lower, first.upper) && row[1].covers_closed(second.lower, second.upper)
            }),
            Some(_) => false,
        }
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn output_within(&self, allowed: &IntegerSet) -> Option<bool> {
        match self.value {
            IntegerReturn::Alias(binding) => {
                let axis = self.axes.iter().position(|known| *known == binding)?;
                Some(self.state.0.iter().all(|row| row[axis].is_subset_of(allowed)))
            }
            IntegerReturn::Const(value) => Some(allowed.contains(value)),
        }
    }

    #[cfg(feature = "internal-checker")]
    fn shows_output_with_bound(
        &self, relation: IntegerRelation, bound: &BigInt, show: Option<&IntegerShowOutput>,
        skip_conjunct: Option<usize>, allowed: &mut Option<IntegerSet>,
    ) -> Option<bool> {
        match self.value {
            IntegerReturn::Const(value) => Some(relation.accepts(value.cmp(bound))
                && show.is_none_or(|show| show.holds_value(value, skip_conjunct))),
            IntegerReturn::Alias(binding) => {
                let axis = self.axes.iter().position(|known| *known == binding)?;
                for row in &self.state.0 {
                    let values = &row[axis];
                    if !values.every_at_extremes(relation, bound) {
                        return Some(false);
                    }
                    let shown = match show {
                        None => true,
                        Some(IntegerShowOutput::Relation { relation, bound }) => values.every_at_extremes(*relation, bound),
                        Some(IntegerShowOutput::Conjunction(atoms)) => atoms.iter().enumerate().all(|(index, (relation, bound))| {
                            skip_conjunct == Some(index) || values.every_at_extremes(*relation, bound)
                        }),
                        Some(show @ (IntegerShowOutput::Disjunction(_) | IntegerShowOutput::Alternatives { .. })) => {
                            let domain = match allowed {
                                Some(domain) => domain,
                                None => allowed.insert(show.normalize_union(self.ty)?),
                            };
                            values.is_subset_of(domain)
                        }
                    };
                    if !shown {
                        return Some(false);
                    }
                }
                Some(true)
            }
        }
    }

    fn shows(self, proof: &IntegerShowProof<'_>) -> Option<bool> {
        let (guard, then_claim, else_claim) = match proof {
            IntegerShowProof::Equality(claim) => return shows_return_equality(&self.state, &self.axes, &self.value, claim),
            IntegerShowProof::Sequence(_) => return None,
            #[cfg(feature = "internal-checker")]
            IntegerShowProof::Output(_) | IntegerShowProof::EntryComparison(_)
            | IntegerShowProof::GuardedOutput { .. } | IntegerShowProof::MixedSequence { .. } => return None,
            IntegerShowProof::Conditional { guard, then_claim, else_claim } => (guard, then_claim, else_claim),
        };
        let (yes, no) = match guard {
            CheckerPredicate::Boolean(true) => (self.state, IntegerInputs::default()),
            CheckerPredicate::Boolean(false) => (IntegerInputs::default(), self.state),
            CheckerPredicate::EntryDomain { .. } | CheckerPredicate::DirectiveDisjunction { .. } | CheckerPredicate::ProofReflexiveBranch(_) => return None,
            CheckerPredicate::Integer(predicate) => {
                let index = self.axes.iter().position(|binding| **binding == predicate.binding)?;
                self.state.split_axis(index, predicate)?
            }
        };
        Some(
            shows_return_equality(&yes, &self.axes, &self.value, then_claim)?
                && shows_return_equality(&no, &self.axes, &self.value, else_claim)?,
        )
    }

}

enum IntegerControl<'a> {
    Continue(IntegerInputs),
    Returned,
    Fail(IntegerCheckFailure<'a>),
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ProofCoverage<T> {
    Applicable(T),
    NoApplicableInputs,
}

#[cfg(test)]
pub(super) fn check_integer_requirements(
    calls: &FunctionCallSites,
) -> Option<Result<(), IntegerCheckFailure<'_>>> {
    check_integer_requirement_activity(calls).map(|result| result.map(drop))
}

pub(super) fn check_integer_requirement_activity(
    calls: &FunctionCallSites,
) -> Option<Result<bool, IntegerCheckFailure<'_>>> {
    run_integer_callable::<false, false, _>(calls, integer_tail_type(calls), None, None, None, None, None, None, false, ())
        .map(|result| result.map(|coverage| matches!(coverage, ProofCoverage::Applicable(true))))
}

pub(super) fn collect_integer_requirement_failures(
    calls: &FunctionCallSites,
) -> Option<(bool, Vec<IntegerCheckFailure<'_>>)> {
    let mut failures = Vec::new();
    let mut sink = |failure| failures.push(failure);
    let (active, _) = collect_integer_requirement_failures_with_calls(calls, IntegerRequirementEntry::Arbitrary, None, false, &mut sink)?;
    Some((active, failures))
}

pub(super) enum IntegerRequirementEntry<'a> {
    Arbitrary,
    Exact(&'a [&'a BigInt]),
    Projected { slot: usize, values: Rc<IntegerSet>, fixed: Option<&'a BigInt> },
}

pub(super) enum IntegerRequirementContinuation {
    Feasible,
    Empty,
    Projected(Rc<IntegerSet>),
}

struct IntegerFailureCollection<'a, 'sink> {
    continuation: IntegerRequirementContinuation,
    projected: Option<(usize, IntegerRow)>,
    allow_projected_calls: bool,
    sink: &'sink mut dyn FnMut(IntegerCheckFailure<'a>),
}

pub(super) fn collect_integer_requirement_failures_with_calls<'a>(
    calls: &'a FunctionCallSites,
    entry: IntegerRequirementEntry<'_>,
    handle_call: Option<&mut ActualCallHandler<'a, '_>>,
    allow_projected_calls: bool,
    sink: &mut dyn FnMut(IntegerCheckFailure<'a>),
) -> Option<(bool, IntegerRequirementContinuation)> {
    let (tail, exact, projected) = match entry {
        IntegerRequirementEntry::Arbitrary => (integer_tail_type(calls), None, None),
        IntegerRequirementEntry::Exact(arguments) => (None, Some(arguments), None),
        IntegerRequirementEntry::Projected { slot, values, fixed } => {
            let row = match (slot, fixed) {
                (0, None) => vec![values],
                (0, Some(fixed)) => vec![values, Rc::new(IntegerSet::singleton(fixed))],
                (1, Some(fixed)) => vec![Rc::new(IntegerSet::singleton(fixed)), values],
                _ => return None,
            };
            (None, None, Some((slot, row)))
        }
    };
    let mut collected = IntegerFailureCollection {
        continuation: IntegerRequirementContinuation::Empty, projected, allow_projected_calls, sink,
    };
    let ProofCoverage::Applicable(active) = run_integer_callable::<false, true, _>(
        calls, tail, None, None, None, exact, handle_call, Some(&mut collected), false, (),
    )?.ok()? else { return None; };
    Some((active, collected.continuation))
}

pub(super) type ExactIntegerArgument<'a> = (&'a ScalarType, &'a BigInt);

pub(super) fn exact_integer_call_argument<'a>(
    structure: &'a OccurrenceStructure,
    call: OccurrenceOrdinal,
    lexical: impl FnOnce(OccurrenceOrdinal, &ScalarType) -> Option<&'a BigInt>,
) -> Option<ExactIntegerArgument<'a>> {
    exact_integer_call_argument_at(structure, call, 0, lexical)
}

pub(super) fn exact_integer_call_argument_at<'a>(
    structure: &'a OccurrenceStructure,
    call: OccurrenceOrdinal,
    slot: usize,
    lexical: impl FnOnce(OccurrenceOrdinal, &ScalarType) -> Option<&'a BigInt>,
) -> Option<ExactIntegerArgument<'a>> {
    let (argument, ty) = checked_integer_call_argument_at(structure, call, slot)?;
    let value = match structure.checker_integer_literals.get(&argument) {
        Some(value) => value,
        None => lexical(argument, ty)?,
    };
    Some((ty, value))
}

fn checked_integer_call_argument_at(
    structure: &OccurrenceStructure, call: OccurrenceOrdinal, slot: usize,
) -> Option<(OccurrenceOrdinal, &ScalarType)> {
    let OccurrenceNode::Expression(argument) = structure.edges.get(&(
        OccurrenceParent::Expression(call), OccurrenceEdge::CallArgument(slot),
    ))? else {
        return None;
    };
    let ty = structure.checked_types.expressions.get(argument)?;
    is_integer_type(ty).then_some((*argument, ty))
}

pub(super) enum ActualCallResult {
    Unit,
    EmptyUnit,
    ProjectedUnit(Rc<IntegerSet>),
    Integer(BigInt),
}

pub(super) struct ProjectedIntegerArgument<'a> {
    pub(super) ty: &'a ScalarType,
    pub(super) caller_axis: usize,
    pub(super) slot: usize,
    path: &'a IntegerInputs,
    pub(super) fixed: Option<ExactIntegerArgument<'a>>,
}

#[cfg(test)]
std::thread_local! {
    pub(super) static PROJECTED_ARGUMENT_MATERIALIZATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl ProjectedIntegerArgument<'_> {
    pub(super) fn materialize(&self) -> Option<Rc<IntegerSet>> {
        #[cfg(test)]
        PROJECTED_ARGUMENT_MATERIALIZATIONS.with(|count| count.set(count.get() + 1));
        self.path.project_axis(self.caller_axis)
    }
}

pub(super) enum NestedIntegerArgument<'a> {
    Single(Option<ExactIntegerArgument<'a>>),
    Pair(ExactIntegerArgument<'a>, ExactIntegerArgument<'a>),
    Projected(ProjectedIntegerArgument<'a>),
}

enum NestedIntegerOperand<'a> {
    Exact(ExactIntegerArgument<'a>),
    Axis(&'a ScalarType, usize),
}

fn nested_integer_operand<'a>(
    calls: &'a FunctionCallSites, expression: OccurrenceOrdinal, slot: usize,
    roots: &IntegerRoots<'_>, path: &'a IntegerInputs,
) -> Option<NestedIntegerOperand<'a>> {
    let (argument, ty) = checked_integer_call_argument_at(&calls.structure, expression, slot)?;
    if let Some(value) = calls.structure.checker_integer_literals.get(&argument) {
        return Some(NestedIntegerOperand::Exact((ty, value)));
    }
    let PlainNameIdentity::Lexical(binding) = calls.prepared_plain_name(argument)? else { return None; };
    let (axis, binding_type) = roots.lookup(binding)?;
    if binding_type != ty {
        return None;
    }
    Some(match path.singleton_at(axis) {
        Some(value) => NestedIntegerOperand::Exact((ty, value)),
        None => NestedIntegerOperand::Axis(ty, axis),
    })
}

fn nested_pair_argument<'a>(
    calls: &'a FunctionCallSites, expression: OccurrenceOrdinal, roots: &IntegerRoots<'_>, path: &'a IntegerInputs,
    allow_projection: bool,
) -> Option<NestedIntegerArgument<'a>> {
    let first = nested_integer_operand(calls, expression, 0, roots, path)?;
    let second = nested_integer_operand(calls, expression, 1, roots, path)?;
    let (ty, caller_axis, slot, fixed) = match (first, second) {
        (NestedIntegerOperand::Exact(first), NestedIntegerOperand::Exact(second)) =>
            return Some(NestedIntegerArgument::Pair(first, second)),
        (NestedIntegerOperand::Axis(ty, axis), NestedIntegerOperand::Exact(fixed)) if allow_projection =>
            (ty, axis, 0, fixed),
        (NestedIntegerOperand::Exact(fixed), NestedIntegerOperand::Axis(ty, axis)) if allow_projection =>
            (ty, axis, 1, fixed),
        _ => return None,
    };
    Some(NestedIntegerArgument::Projected(ProjectedIntegerArgument { ty, caller_axis, slot, path, fixed: Some(fixed) }))
}

fn actual_nested_argument<'a>(
    calls: &'a FunctionCallSites,
    expression: OccurrenceOrdinal,
    roots: &IntegerRoots<'_>,
    path: &'a IntegerInputs,
    allow_projection: bool,
) -> NestedIntegerArgument<'a> {
    match nested_integer_operand(calls, expression, 0, roots, path) {
        Some(NestedIntegerOperand::Exact(argument)) => NestedIntegerArgument::Single(Some(argument)),
        Some(NestedIntegerOperand::Axis(ty, caller_axis)) if allow_projection =>
            NestedIntegerArgument::Projected(ProjectedIntegerArgument {
                ty, caller_axis, slot: 0, path, fixed: None,
            }),
        _ => NestedIntegerArgument::Single(None),
    }
}

pub(super) type ActualCallHandler<'a, 'handler> =
    dyn FnMut(
        &'a FunctionCallSites, OccurrenceOrdinal, &CheckedCallSignature, NestedIntegerArgument<'_>, &ScalarType,
    ) -> Option<ActualCallResult> + 'handler;

pub(super) fn check_integer_requirements_with_calls<'a>(
    calls: &'a FunctionCallSites,
    exact_arguments: Option<&[&BigInt]>,
    handle_call: &mut ActualCallHandler<'a, '_>,
) -> Option<Result<bool, IntegerCheckFailure<'a>>> {
    let tail = if exact_arguments.is_none() { integer_tail_type(calls) } else { None };
    run_integer_callable::<false, false, _>(calls, tail, None, None, None, exact_arguments, Some(handle_call), None, false, ())
        .map(|result| result.map(|coverage| matches!(coverage, ProofCoverage::Applicable(true))))
}

pub(super) fn proves_requirement_with_imported_root_local<'a>(
    calls: &'a FunctionCallSites,
    local: ImportedLocal<'a>,
) -> Option<bool> {
    if matches!(&local.values, ImportedIntegerValues::Ready(values, _) if values.is_empty()) {
        return None;
    }
    match run_integer_callable::<true, false, _>(calls, integer_tail_type(calls), None, None, Some(local), None, None, None, false, ())? {
        Ok(ProofCoverage::Applicable(true)) => Some(true),
        Ok(ProofCoverage::Applicable(false)) => None,
        Ok(ProofCoverage::NoApplicableInputs) => None,
        Err(_) => Some(false),
    }
}

#[cfg(test)]
pub(super) fn observe_integer_returns(
    calls: &FunctionCallSites,
) -> Option<Result<Vec<IntegerReturningPath<'_>>, IntegerCheckFailure<'_>>> {
    match observe_integer_returns_in_scope(calls, None, None, false, ())? {
        Ok(ProofCoverage::Applicable(paths)) => Some(Ok(paths)),
        Ok(ProofCoverage::NoApplicableInputs) => None,
        Err(failure) => Some(Err(failure)),
    }
}

#[cfg(feature = "internal-checker")]
pub(super) fn observe_arbitrary_integer_returns<'a>(
    calls: &'a FunctionCallSites,
    entry_filter: Option<&[CheckerPredicate]>,
) -> Option<Result<ProofCoverage<Vec<IntegerReturningPath<'a>>>, IntegerCheckFailure<'a>>> {
    observe_integer_returns_in_scope(calls, entry_filter, None, false, ())
}

#[cfg(feature = "internal-checker")]
pub(super) fn observe_unused_module_proof_returns<'a>(
    calls: &'a FunctionCallSites,
    entry_filter: Option<&[CheckerPredicate]>,
) -> Option<Result<ProofCoverage<Vec<IntegerReturningPath<'a>>>, IntegerCheckFailure<'a>>> {
    observe_integer_returns_in_scope(calls, entry_filter, None, true, ())
}

#[cfg(all(test, feature = "internal-checker"))]
thread_local! {
    static INTEGER_OBSERVATIONS: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}

#[cfg(all(test, feature = "internal-checker"))]
pub(super) fn take_integer_observations() -> (usize, usize) {
    INTEGER_OBSERVATIONS.with(|observations| observations.replace((0, 0)))
}

fn observe_integer_returns_in_scope<'a, Authored: ScopedAuthoredReturn<'a>>(
    calls: &'a FunctionCallSites,
    entry_filter: Option<&[CheckerPredicate]>,
    exact_arguments: Option<&[&BigInt]>,
    proof_reflexive_branches: bool,
    authored_return: Authored,
) -> Option<Result<ProofCoverage<Vec<IntegerReturningPath<'a>>>, IntegerCheckFailure<'a>>> {
    let tail = integer_tail_type(calls)?;
    #[cfg(all(test, feature = "internal-checker"))]
    INTEGER_OBSERVATIONS.with(|observations| {
        let (arbitrary, actual) = observations.get();
        observations.set(if exact_arguments.is_some() { (arbitrary, actual + 1) } else { (arbitrary + 1, actual) });
    });
    let mut paths = Vec::new();
    match run_integer_callable::<false, false, _>(calls, Some(tail), entry_filter, Some(&mut paths), None, exact_arguments, None, None, proof_reflexive_branches, authored_return)? {
        Ok(ProofCoverage::Applicable(_)) if !paths.is_empty() => {
            Some(Ok(ProofCoverage::Applicable(paths)))
        }
        Ok(ProofCoverage::Applicable(_)) => None,
        Ok(ProofCoverage::NoApplicableInputs) => Some(Ok(ProofCoverage::NoApplicableInputs)),
        Err(failure) => Some(Err(failure)),
    }
}

pub(super) fn exact_integer_return_at_entry(
    calls: &FunctionCallSites,
    expected: &ScalarType,
    arguments: &[&BigInt],
) -> Option<BigInt> {
    if integer_tail_type(calls)? != expected {
        return None;
    }
    let ProofCoverage::Applicable(paths) = observe_integer_returns_in_scope(calls, None, Some(arguments), false, ())?.ok()? else {
        return None;
    };
    fn singleton<'a>(path: &'a IntegerReturningPath<'_>) -> Option<&'a BigInt> {
        match &path.value {
            IntegerReturn::Const(value) => Some(value),
            IntegerReturn::Alias(binding) => {
                let axis = path.axes.iter().position(|id| *id == *binding)?;
                path.state.singleton_at(axis)
            }
        }
    }
    let (first, rest) = paths.split_first()?;
    let value = singleton(first)?;
    rest.iter().all(|path| singleton(path) == Some(value)).then(|| value.clone())
}

#[cfg(test)]
pub(super) fn check_integer_return_bound<'a>(
    calls: &'a FunctionCallSites,
    operator: &BinaryOperator,
    bound: &BigInt,
) -> Option<Result<bool, IntegerCheckFailure<'a>>> {
    match check_integer_return_bound_in_scope(calls, operator, bound, None, None)? {
        Ok(ProofCoverage::Applicable(holds)) => Some(Ok(holds)),
        Ok(ProofCoverage::NoApplicableInputs) => None,
        Err(failure) => Some(Err(failure)),
    }
}

#[cfg(test)]
pub(super) fn check_integer_return_bound_where<'a>(
    calls: &'a FunctionCallSites,
    operator: &BinaryOperator,
    bound: &BigInt,
    entry_filter: &[CheckerPredicate],
) -> Option<Result<ProofCoverage<bool>, IntegerCheckFailure<'a>>> {
    check_integer_return_bound_in_scope(calls, operator, bound, Some(entry_filter), None)
}

pub(super) fn check_integer_return_bound_with_shows<'a>(
    calls: &'a FunctionCallSites,
    operator: &BinaryOperator,
    bound: &BigInt,
    entry_filter: Option<&[CheckerPredicate]>,
    proof: &IntegerShowProof<'_>,
) -> Option<Result<ProofCoverage<bool>, IntegerCheckFailure<'a>>> {
    check_integer_return_bound_in_scope(calls, operator, bound, entry_filter, Some(proof))
}

fn check_integer_return_bound_in_scope<'a>(
    calls: &'a FunctionCallSites,
    operator: &BinaryOperator,
    bound: &BigInt,
    entry_filter: Option<&[CheckerPredicate]>,
    proof: Option<&IntegerShowProof<'_>>,
) -> Option<Result<ProofCoverage<bool>, IntegerCheckFailure<'a>>> {
    let relation = IntegerRelation::from_operator(operator)?;
    let paths = match observe_integer_returns_in_scope(calls, entry_filter, None, false, ())? {
        Ok(ProofCoverage::Applicable(paths)) => paths,
        Ok(ProofCoverage::NoApplicableInputs) => return Some(Ok(ProofCoverage::NoApplicableInputs)),
        Err(failure) => return Some(Err(failure)),
    };
    if let Some(proof @ IntegerShowProof::Conditional { .. }) = proof {
        return Some(Ok(ProofCoverage::Applicable(
            evaluate_owning_show_conditional(paths, relation, bound, proof)?,
        )));
    }
    Some(Ok(ProofCoverage::Applicable(evaluate_borrowed_return_proof(&paths, relation, bound, proof)?)))
}

pub(super) fn evaluate_owning_show_conditional(
    paths: Vec<IntegerReturningPath<'_>>, relation: IntegerRelation, bound: &BigInt, proof: &IntegerShowProof<'_>,
) -> Option<bool> {
    for path in paths {
        if !path.output_satisfies(relation, bound)? || !path.shows(proof)? {
            return Some(false);
        }
    }
    Some(true)
}

#[cfg(feature = "internal-checker")]
pub(super) fn observe_authored_show_returns<'a>(
    calls: &'a FunctionCallSites, entry_filter: Option<&[CheckerPredicate]>,
    body: &'a super::ScalarBlock, value: &'a std::cell::OnceCell<Option<BigInt>>,
) -> Option<Result<ProofCoverage<Vec<IntegerReturningPath<'a>>>, IntegerCheckFailure<'a>>> {
    observe_integer_returns_in_scope(calls, entry_filter, None, false, NamedAuthoredReturn { body, value })
}

#[cfg(feature = "internal-checker")]
pub(super) fn evaluate_mixed_entry_output(
    paths: &[IntegerReturningPath<'_>], formals: &[ReferenceBindingId],
    output: &IntegerShowConstraints<'_>, entry: &[Option<IntegerRelation>; 2],
) -> Option<bool> {
    let mut proved = true;
    let inactive = IntegerRelation::from_operator(&BinaryOperator::Equal)?;
    let mut typed_entry = [(0, inactive, false, CartesianRelation::Never); 2];
    let mut active = 0;
    for (axis, condition) in entry.iter().enumerate() {
        let Some(condition) = condition else { continue; };
        typed_entry[active] = (axis, condition.reversed(), condition.accepts(std::cmp::Ordering::Equal),
            CartesianRelation::prepare(*condition));
        active += 1;
    }
    let typed_entry = &typed_entry[..active];
    let uses_entry = !typed_entry.is_empty();
    for path in paths {
        let returned = match path.value {
            IntegerReturn::Alias(alias) => {
                if uses_entry && !formals.contains(alias) {
                    return None;
                }
                if !proved { continue; }
                Some(path.axes.iter().position(|known| *known == alias)?)
            }
            IntegerReturn::Const(value) => {
                if !proved { continue; }
                if !output.holds_value(value) {
                    proved = false;
                    continue;
                }
                if !uses_entry {
                    continue;
                }
                None
            }
        };
        if uses_entry && !path.axes.get(..formals.len())?.iter().copied().eq(formals.iter()) {
            return None;
        }
        for row in &path.state.0 {
            if returned.is_some_and(|axis| !output.holds_set(&row[axis])) {
                proved = false;
                break;
            }
            if !uses_entry {
                continue;
            }
            for &(axis, reversed, reflexive, comparison) in typed_entry {
                let old = row.get(axis)?;
                let holds = match (path.value, returned) {
                    (IntegerReturn::Const(value), _) => old.every_at_extremes(reversed, value),
                    (_, Some(returned)) if returned == axis => reflexive,
                    (_, Some(returned)) => comparison.holds(&row[returned], old)?,
                    _ => return None,
                };
                if !holds {
                    proved = false;
                    break;
                }
            }
            if !proved {
                break;
            }
        }
    }
    Some(proved)
}

pub(super) fn evaluate_borrowed_return_proof(
    paths: &[IntegerReturningPath<'_>], relation: IntegerRelation, bound: &BigInt, proof: Option<&IntegerShowProof<'_>>,
) -> Option<bool> {
    if let Some(IntegerShowProof::Sequence(claims)) = proof {
        let mut proved = true;
        for path in paths {
            if proved {
                proved = path.output_satisfies(relation, bound)?;
            }
            for claim in claims {
                if proved {
                    proved = shows_return_equality(&path.state, &path.axes, &path.value, claim)?;
                } else if matches!((&path.value, claim),
                    (IntegerReturn::Alias(actual), IntegerReturn::Alias(expected)) if actual != expected)
                {
                    return None;
                }
            }
        }
        return Some(proved);
    }
    #[cfg(feature = "internal-checker")]
    if let Some(IntegerShowProof::MixedSequence { formals, output, entry }) = proof {
        return evaluate_mixed_entry_output(paths, formals, output, entry);
    }
    #[cfg(feature = "internal-checker")]
    if let Some(IntegerShowProof::EntryComparison(entry)) = proof {
        return entry.holds(paths, Some((relation, bound)));
    }
    #[cfg(feature = "internal-checker")]
    if let Some(IntegerShowProof::GuardedOutput { guard, claims }) = proof {
        let guard_relation = IntegerRelation::from_operator(&guard.operator)?;
        let guard_cut = (guard_relation, &guard.bound);
        let else_cut = (guard_relation.complement(), &guard.bound);
        let mut literal_cuts = None;
        let mut proved = true;
        for path in paths {
            if matches!(path.value, IntegerReturn::Alias(alias) if *alias != guard.binding) {
                return None;
            }
            if !proved {
                continue;
            }
            let axis = path.axes.iter().position(|binding| **binding == guard.binding)?;
            let rows = &path.state.0;
            let arms_hold = |yes_ok, no_ok| match (yes_ok, no_ok) {
                (true, true) => true,
                (false, false) => false,
                _ => {
                    let allowed = if yes_ok { guard_cut.0 } else { else_cut.0 };
                    rows.iter().all(|row| row[axis].every_at_extremes(allowed, guard_cut.1))
                }
            };
            proved = match (path.value, claims) {
                (IntegerReturn::Alias(_), GuardedOutputClaims::Literal { yes, no }) =>
                    rows.iter().all(|row| row[axis].intervals().all(|(first, last)| {
                        relation.accepts_closed(first, last, bound) && {
                            let (yes_cut, no_cut) = literal_cuts.get_or_insert_with(||
                                forbidden_guarded_cuts(guard_cut, else_cut, (yes.0, &yes.1), (no.0, &no.1)));
                            !yes_cut.hits(first, last) && !no_cut.hits(first, last)
                        }
                    })),
                (IntegerReturn::Const(value), GuardedOutputClaims::Literal { yes, no }) =>
                    relation.accepts(value.cmp(bound))
                        && arms_hold(yes.0.accepts(value.cmp(&yes.1)), no.0.accepts(value.cmp(&no.1))),
                (IntegerReturn::Alias(_), GuardedOutputClaims::ImmutableEntry { yes, no }) => {
                    use std::cmp::Ordering::Equal;
                    let branch = match (yes.accepts(Equal), no.accepts(Equal)) {
                        (true, true) => None,
                        (true, false) => Some(guard_relation),
                        (false, true) => Some(else_cut.0),
                        (false, false) => {
                            proved = false;
                            continue;
                        }
                    };
                    let joint = (guard_cut.1 == bound).then(|| branch.map_or(relation, |arm| relation.intersection(arm)));
                    rows.iter().all(|row| row[axis].intervals().all(|(first, last)| {
                        if let Some(joint) = joint {
                            joint.accepts_closed(first, last, bound)
                        } else {
                            relation.accepts_closed(first, last, bound)
                                && branch.is_none_or(|allowed| allowed.accepts_closed(first, last, guard_cut.1))
                        }
                    }))
                }
                (IntegerReturn::Const(value), GuardedOutputClaims::ImmutableEntry { yes, no }) => {
                    if !relation.accepts(value.cmp(bound)) {
                        false
                    } else {
                        let (yes_cut, no_cut) = forbidden_guarded_cuts(guard_cut, else_cut, (yes.reversed(), value), (no.reversed(), value));
                        rows.iter().all(|row| row[axis].intervals().all(|(first, last)|
                            !yes_cut.hits(first, last) && !no_cut.hits(first, last)))
                    }
                }
            };
        }
        return Some(proved);
    }
    #[cfg(feature = "internal-checker")]
    if let Some(IntegerShowProof::Output(show)) = proof {
        let (relation, additional, skip_conjunct) = match show {
            IntegerShowOutput::Relation { relation: other, bound: other_bound } if other_bound == bound =>
                (relation.intersection(*other), None, None),
            IntegerShowOutput::Conjunction(atoms) => {
                let index = atoms.binary_search_by(|(_, literal)| literal.cmp(bound)).ok();
                let combined = index.map_or(relation, |index| relation.intersection(atoms[index].0));
                (combined, Some(show), index)
            }
            _ => (relation, Some(show), None),
        };
        let mut allowed = None;
        for path in paths {
            if !path.shows_output_with_bound(relation, bound, additional, skip_conjunct, &mut allowed)? {
                return Some(false);
            }
        }
        return Some(true);
    }
    for path in paths {
        if !path.output_satisfies(relation, bound)? {
            return Some(false);
        }
        match proof {
            None => {}
            Some(IntegerShowProof::Equality(claim)) => {
                if !shows_return_equality(&path.state, &path.axes, &path.value, claim)? {
                    return Some(false);
                }
            }
            _ => return None,
        }
    }
    Some(true)
}

fn integer_tail_type(calls: &FunctionCallSites) -> Option<&ScalarType> {
    let [result] = calls.boundary.results.as_slice() else {
        return None;
    };
    let [output] = calls.boundary.root_outputs.as_slice() else {
        return None;
    };
    (is_integer_type(&result.ty) && result.ty == output.ty).then_some(&result.ty)
}

fn run_integer_callable<'a, const USE_IMPORTED_BOUND: bool, const COLLECT: bool, Authored: ScopedAuthoredReturn<'a>>(
    calls: &'a FunctionCallSites,
    tail: Option<&'a ScalarType>,
    entry_filter: Option<&[CheckerPredicate]>,
    returns: Option<&mut Vec<IntegerReturningPath<'a>>>,
    imported_local: Option<ImportedLocal<'a>>,
    exact_arguments: Option<&[&BigInt]>,
    mut handle_call: Option<&mut ActualCallHandler<'a, '_>>,
    mut failures: Option<&mut IntegerFailureCollection<'a, '_>>,
    proof_reflexive_branches: bool,
    authored_return: Authored,
) -> Option<Result<ProofCoverage<bool>, IntegerCheckFailure<'a>>> {
    let boundary = &calls.boundary;
    let checker_parameters = boundary.checker_parameter_bindings.as_deref()?;
    debug_assert_eq!(boundary.parameters.len(), checker_parameters.len());
    let mut roots = IntegerRoots {
        parameters: boundary.parameters.len(),
        imported_local,
        proof_reflexive_branches,
        ..IntegerRoots::default()
    };
    for (binding, parameter) in checker_parameters.iter().zip(&boundary.parameters) {
        roots.add(binding, &parameter.ty);
    }
    let projected = if COLLECT { failures.as_deref_mut()?.projected.take() } else { None };
    let projected_slot = projected.as_ref().map(|(slot, _)| *slot);
    let mut row: IntegerRow = if let Some((slot, row)) = projected {
        if row.len() != boundary.parameters.len() || row.get(slot)?.is_empty() {
            return None;
        }
        row
    } else if let Some(arguments) = exact_arguments {
        if arguments.len() != boundary.parameters.len() {
            return None;
        }
        arguments.iter().map(|value| Rc::new(IntegerSet::singleton(value))).collect()
    } else {
        boundary.parameters.iter()
            .map(|parameter| IntegerSet::from_type(&parameter.ty).map(Rc::new))
            .collect::<Option<IntegerRow>>()?
    };
    for filter in entry_filter.into_iter().flatten() {
        match filter {
            CheckerPredicate::Boolean(true) => {}
            CheckerPredicate::Boolean(false) => return Some(Ok(ProofCoverage::NoApplicableInputs)),
            CheckerPredicate::DirectiveDisjunction { .. } | CheckerPredicate::ProofReflexiveBranch(_) => return None,
            CheckerPredicate::EntryDomain { binding, values } => {
                let (axis, _) = roots.lookup(binding)?;
                match row[axis].intersection(values) {
                    IntegerIntersection::Unchanged => {}
                    IntegerIntersection::Empty => return Some(Ok(ProofCoverage::NoApplicableInputs)),
                    IntegerIntersection::Changed(filtered) => row[axis] = Rc::new(filtered),
                }
            }
            CheckerPredicate::Integer(predicate) => {
                let (axis, ty) = roots.lookup(&predicate.binding)?;
                if ty != &predicate.ty {
                    return None;
                }
                let kept = row[axis].keep(&predicate.operator, &predicate.bound)?;
                if kept.is_empty() {
                    return Some(Ok(ProofCoverage::NoApplicableInputs));
                }
                row[axis] = Rc::new(kept);
            }
        }
    }
    let result = walk_integer_block::<USE_IMPORTED_BOUND, COLLECT, _>(calls, &mut roots, calls.structure.root, IntegerInputs(vec![row]), tail, returns, &mut handle_call, &mut failures, authored_return)?;
    if USE_IMPORTED_BOUND && roots.imported_local.is_some() {
        return None;
    }
    if COLLECT {
        failures.as_deref_mut()?.continuation = match &result {
            IntegerControl::Continue(path) if path.is_unreachable() => IntegerRequirementContinuation::Empty,
            IntegerControl::Continue(path) => match projected_slot {
                Some(slot) => IntegerRequirementContinuation::Projected(path.project_axis(slot)?),
                None => IntegerRequirementContinuation::Feasible,
            },
            IntegerControl::Returned if projected_slot.is_some() => return None,
            IntegerControl::Returned => IntegerRequirementContinuation::Feasible,
            IntegerControl::Fail(_) => IntegerRequirementContinuation::Empty,
        };
    }
    match result {
        IntegerControl::Continue(_) | IntegerControl::Returned => {
            Some(Ok(ProofCoverage::Applicable(roots.reached_obligation)))
        }
        IntegerControl::Fail(failure) => Some(Err(failure)),
    }
}

#[cfg(all(test, feature = "internal-checker"))]
thread_local! {
    pub(super) static DIRECTIVE_DISJUNCTION_NORMALIZATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn normalize_directive_disjunction(
    operation: CheckerOperation, ty: &ScalarType,
    [(lhs, left), (rhs, right)]: &[(IntegerRelation, BigInt); 2],
) -> Option<IntegerSet> {
    #[cfg(all(test, feature = "internal-checker"))]
    DIRECTIVE_DISJUNCTION_NORMALIZATIONS.with(|count| count.set(count.get() + 1));
    match operation {
        CheckerOperation::Require => Some(IntegerSet::from_type(ty)?
            .keep_relation(lhs.complement(), left).keep_relation(rhs.complement(), right)),
        CheckerOperation::Narrow => IntegerSet::comparison_union(ty, (*lhs, left), (*rhs, right)),
    }
}

fn walk_integer_block<'a, const USE_IMPORTED_BOUND: bool, const COLLECT: bool, Authored: ScopedAuthoredReturn<'a>>(
    calls: &'a FunctionCallSites,
    roots: &mut IntegerRoots<'a>,
    block: BlockOrdinal,
    mut path: IntegerInputs,
    tail: Option<&'a ScalarType>,
    mut returns: Option<&mut Vec<IntegerReturningPath<'a>>>,
    handle_call: &mut Option<&mut ActualCallHandler<'a, '_>>,
    failures: &mut Option<&mut IntegerFailureCollection<'a, '_>>,
    authored_return: Authored,
) -> Option<IntegerControl<'a>> {
    let depth = roots.axes.len();
    let structure = &calls.structure;
    for item in &structure.blocks.get(block.0)?.items {
        if path.is_unreachable() {
            break;
        }
        match item.kind {
            BlockItemKind::Evaluation { expression } => {
                if let Some(directive) = structure.checker_directives.get(&expression) {
                    let predicate = directive.predicate.as_ref()?;
                    roots.reached_obligation = true;
                    if let (true, CheckerPredicate::DirectiveDisjunction { binding, ty, atoms }) = (COLLECT, predicate) {
                        let (axis, actual_type) = roots.lookup(binding)?;
                        if ty != actual_type {
                            return None;
                        }
                        let values = normalize_directive_disjunction(directive.operation, ty, atoms)?;
                        let failing = match directive.operation {
                            CheckerOperation::Require => path.intersection_witness(axis, &values),
                            CheckerOperation::Narrow => path.restrict_axis_or_witness(axis, &values),
                        };
                        if !failing.is_unreachable() {
                            (failures.as_deref_mut()?.sink)(IntegerCheckFailure {
                                directive: expression, state: failing, axes: roots.axes.clone(),
                            });
                        }
                        continue;
                    }
                    if COLLECT && directive.operation == CheckerOperation::Require {
                        let failing = path.failing_witness(roots, predicate)?;
                        if !failing.is_unreachable() {
                            (failures.as_deref_mut()?.sink)(IntegerCheckFailure {
                                directive: expression, state: failing, axes: roots.axes.clone(),
                            });
                        }
                        continue;
                    }
                    let (passing, failing, _) = path.split(roots, predicate)?;
                    if directive.operation.rejects(!passing.is_unreachable(), !failing.is_unreachable()) {
                        let failure = IntegerCheckFailure {
                            directive: expression, state: failing, axes: roots.axes.clone(),
                        };
                        if COLLECT {
                            (failures.as_deref_mut()?.sink)(failure);
                        } else {
                            return Some(IntegerControl::Fail(failure));
                        }
                    }
                    path = passing;
                } else if structure.checker_direct_targets.contains_key(&expression) {
                    let handle = handle_call.as_mut()?;
                    let signature = calls.checked_call_signatures.get(&expression)?;
                    let project = COLLECT && failures.as_ref().is_some_and(|collection| collection.allow_projected_calls);
                    let argument = if COLLECT && signature.parameters.len() == 2 {
                        nested_pair_argument(calls, expression, roots, &path, project)?
                    } else {
                        actual_nested_argument(calls, expression, roots, &path, project)
                    };
                    let projected = match &argument {
                        NestedIntegerArgument::Projected(projection) => Some(projection.caller_axis),
                        _ => None,
                    };
                    match handle(calls, expression, signature, argument, &ScalarType::Unit)? {
                        ActualCallResult::Unit => {}
                        ActualCallResult::EmptyUnit if COLLECT => path = IntegerInputs::default(),
                        ActualCallResult::ProjectedUnit(survivors) if COLLECT => {
                            path.restrict_axis(projected?, &survivors);
                        }
                        _ => return None,
                    }
                } else {
                    let (condition, then_block, else_block) = match calls.conditionals.get(&expression)? {
                        ConditionalRegion::Unit { condition, then_block, .. } =>
                            (condition, *then_block, None),
                        ConditionalRegion::Value { condition, then_block, else_block, .. }
                            if structure.checked_types.expressions.get(&expression) == Some(&ScalarType::Unit) =>
                            (condition, *then_block, Some(*else_block)),
                        _ => return None,
                    };
                    let predicate = structure.checker_branches.get(condition)?;
                    let (yes, no, axis) = path.split(roots, predicate)?;
                    let mut continuation: Option<IntegerInputs> = None;
                    for (arm, incoming) in [(Some(then_block), yes), (else_block, no)] {
                        if incoming.is_unreachable() {
                            continue;
                        }
                        let control = if let Some(arm) = arm {
                            walk_integer_block::<USE_IMPORTED_BOUND, COLLECT, _>(calls, roots, arm, incoming, None, None, handle_call, failures, authored_return)?
                        } else {
                            IntegerControl::Continue(incoming)
                        };
                        match control {
                            IntegerControl::Continue(outgoing) => {
                                continuation = Some(match continuation {
                                    Some(previous) => previous.join(outgoing, axis),
                                    None => outgoing,
                                });
                            }
                            failure @ IntegerControl::Fail(_) => return Some(failure),
                            IntegerControl::Returned => return None,
                        }
                    }
                    path = continuation.unwrap_or_default();
                }
            }
            BlockItemKind::FinalOutput(slot) => {
                let ty = structure.checked_types.expressions.get(&slot.expression)?;
                if let Some(expected) = tail {
                    if slot.output != 0 || ty != expected {
                        return None;
                    }
                } else if !is_integer_type(ty) {
                    return None;
                }
                if let Some(ConditionalRegion::Value { condition, then_block, else_block, .. }) =
                    calls.conditionals.get(&slot.expression)
                {
                    let expected = tail?;
                    let predicate = structure.checker_branches.get(condition)?;
                    let (yes, no, _) = path.split(roots, predicate)?;
                    for (arm, incoming) in [(*then_block, yes), (*else_block, no)] {
                        if incoming.is_unreachable() {
                            continue;
                        }
                        match walk_integer_block::<USE_IMPORTED_BOUND, COLLECT, _>(calls, roots, arm, incoming, Some(expected), returns.as_deref_mut(), handle_call, failures, authored_return)? {
                            IntegerControl::Returned => {}
                            failure @ IntegerControl::Fail(_) => return Some(failure),
                            IntegerControl::Continue(outgoing) if COLLECT && outgoing.is_unreachable() => {}
                            IntegerControl::Continue(_) => return None,
                        }
                    }
                    roots.truncate(depth);
                    return Some(IntegerControl::Returned);
                }
                let value = if let Some(value) = structure.checker_integer_literals.get(&slot.expression) {
                    IntegerReturn::Const(value)
                } else if let Some(PlainNameIdentity::Lexical(binding)) = calls.prepared_plain_name(slot.expression) {
                    let (_, binding_type) = roots.lookup(binding)?;
                    if binding_type != ty {
                        return None;
                    }
                    IntegerReturn::Alias(binding)
                } else if Authored::ENABLED && tail.is_some() {
                    IntegerReturn::Const(authored_return.at(structure, block, slot, ty)?)
                } else {
                    return None;
                };
                if tail.is_some() {
                    if let Some(returns) = returns.as_deref_mut() {
                        returns.push(IntegerReturningPath {
                            value,
                            ty,
                            state: path,
                            axes: roots.axes.iter().map(|(binding, _)| *binding).collect(),
                        });
                    }
                    roots.truncate(depth);
                    return Some(IntegerControl::Returned);
                }
            }
            BlockItemKind::Assignment { group } => {
                let (axis, value) = integer_assignment(calls, roots, group)?;
                path = path.overwrite(axis, value);
            }
            BlockItemKind::Binding(ref binding) => {
                if USE_IMPORTED_BOUND && block == structure.root
                    && roots.imported_local.as_ref().is_some_and(|local| local.position == item.position)
                {
                    let local = roots.imported_local.take()?;
                    let origin = structure.checker_integer_call_locals.get(&(block, item.position))?;
                    let [receiver] = binding.receivers() else {
                        return None;
                    };
                    #[derive(Clone, Copy)]
                    enum Argument<'a> {
                        Literal(&'a BigInt),
                        Axis(usize),
                    }
                    let mut arguments = [None; 2];
                    for (slot, (ordinal, argument)) in origin.arguments.as_slice().iter().enumerate() {
                        arguments[slot] = Some(match argument {
                            CheckerIntegerCallArgument::Literal(value) => Argument::Literal(value),
                            CheckerIntegerCallArgument::PlainName => {
                                let PlainNameIdentity::Lexical(argument) = calls.prepared_plain_name(*ordinal)? else {
                                    return None;
                                };
                                let (axis, ty) = roots.lookup(argument)?;
                                if structure.checked_types.expressions.get(ordinal)? != ty {
                                    return None;
                                }
                                Argument::Axis(axis)
                            }
                        });
                    }
                    let applies = local.guards.iter().all(|guard| match arguments.get(guard.slot).copied().flatten() {
                        Some(Argument::Literal(value)) => guard.relation.accepts(value.cmp(guard.bound)),
                        Some(Argument::Axis(axis)) => path.0.iter().all(|row| row[axis].every(guard.relation, guard.bound)),
                        None => false,
                    });
                    if !applies {
                        return None;
                    }
                    let values = match local.values {
                        ImportedIntegerValues::Ready(values, _) => values,
                        #[cfg(feature = "internal-checker")]
                        ImportedIntegerValues::Deferred { slot, domain, issue } => {
                            let admitted = !path.is_unreachable() && match arguments.get(slot).copied().flatten() {
                                Some(Argument::Literal(value)) => domain.contains(value),
                                Some(Argument::Axis(axis)) => path.0.iter().all(|row| row[axis].is_subset_of(domain)),
                                None => false,
                            };
                            if !admitted {
                                return None;
                            }
                            issue().filter(|values| !values.is_empty())?
                        }
                    };
                    roots.add(&origin.binding, &receiver.ty);
                    path.append_local(values);
                    continue;
                }
                if let Some(origin) = structure.checker_integer_call_locals.get(&(block, item.position)) {
                    let handle = handle_call.as_mut()?;
                    let [receiver] = binding.receivers() else {
                        return None;
                    };
                    let signature = calls.checked_call_signatures.get(&origin.call)?;
                    let argument = actual_nested_argument(calls, origin.call, roots, &path, false);
                    let ActualCallResult::Integer(value) = handle(calls, origin.call, signature, argument, &receiver.ty)? else {
                        return None;
                    };
                    roots.add(&origin.binding, &receiver.ty);
                    path.append_local(IntegerSet::singleton(&value));
                    continue;
                }
                let (identity, ty, value) = integer_local(calls, binding, block, item.position)?;
                roots.add(identity, ty);
                path.append_local(IntegerSet::singleton(value));
            }
        }
    }
    if roots.axes.len() != depth {
        path.drop_locals(depth);
        roots.truncate(depth);
    }
    Some(IntegerControl::Continue(path))
}

fn integer_assignment<'a>(
    calls: &'a FunctionCallSites,
    roots: &IntegerRoots<'_>,
    position: BlockPosition,
) -> Option<(usize, &'a BigInt)> {
    let group = calls.assignments.get(&(position.block, position.item))?;
    let [rhs] = group.rhs_evaluations().as_slice() else {
        return None;
    };
    let [destination] = group.destinations().as_slice() else {
        return None;
    };
    if group.projected_outputs().len() != 0 {
        return None;
    }
    let DirectDestinationIdentity::Existing(binding) =
        calls.prepared_direct_destination(destination.destination)?
    else {
        return None;
    };
    let (axis, ty) = roots.lookup(binding)?;
    if axis >= roots.parameters {
        return None;
    }
    let AssignmentReplacement::Resolved(replacement) = &destination.replacement else {
        return None;
    };
    if ty != replacement {
        return None;
    }
    let value = calls.structure.checker_integer_literals.get(&rhs.expression)?;
    Some((axis, value))
}

fn integer_local<'a>(
    calls: &'a FunctionCallSites,
    region: &'a BindingRegion,
    block: BlockOrdinal,
    item: usize,
) -> Option<(&'a ReferenceBindingId, &'a ScalarType, &'a BigInt)> {
    let identity = calls.structure.checker_integer_locals.get(&(block, item))?;
    let BindingInitialization::Outputs(outputs) = &region.initialization else {
        return None;
    };
    let [receiver] = region.receivers() else {
        return None;
    };
    let value = calls.structure.checker_integer_literals.get(&outputs.primary)?;
    Some((identity, &receiver.ty, value))
}


#[test]
fn actual_call_projection_accepts_only_the_same_singleton_in_every_row() {
    let one = BigInt::from(1);
    let two = BigInt::from(2);
    let coordinate = |value| Rc::new(IntegerSet::singleton(value));
    let same = IntegerInputs(vec![vec![coordinate(&one)], vec![coordinate(&one)]]);
    assert_eq!(same.singleton_at(0), Some(&one));
    let distinct = IntegerInputs(vec![vec![coordinate(&one)], vec![coordinate(&two)]]);
    assert_eq!(distinct.singleton_at(0), None);
    let broad = IntegerInputs(vec![vec![Rc::new(IntegerSet::from_type(&ScalarType::U8).unwrap())]]);
    assert_eq!(broad.singleton_at(0), None);
    assert_eq!(IntegerInputs::default().singleton_at(0), None);
}

#[test]
fn projected_axis_restriction_reuses_unchanged_rc_and_preserves_correlated_rows() {
    let range = |first: u8, last: u8| {
        IntegerSet::from_type(&ScalarType::U8).unwrap()
            .keep(&BinaryOperator::GreaterEqual, &BigInt::from(first)).unwrap()
            .keep(&BinaryOperator::LessEqual, &BigInt::from(last)).unwrap()
    };
    let untouched = Rc::new(range(1, 2));
    let to_change = Rc::new(range(4, 10));
    let to_drop = Rc::new(range(12, 15));
    let untouched_other = Rc::new(range(50, 50));
    let changed_other = Rc::new(range(60, 60));
    let dropped_other = Rc::new(range(70, 70));
    let survivors = Rc::new(range(1, 2).join(range(5, 5)).join(range(7, 8)));
    assert!(!Rc::ptr_eq(&untouched, &survivors));
    let mut input = IntegerInputs(vec![
        vec![Rc::clone(&untouched), Rc::clone(&untouched_other)],
        vec![Rc::clone(&to_change), Rc::clone(&changed_other)],
        vec![Rc::clone(&to_drop), Rc::clone(&dropped_other)],
    ]);
    input.restrict_axis(0, &survivors);
    let [first, second] = input.0.as_slice() else { panic!("expected two retained rows"); };
    assert!(Rc::ptr_eq(&first[0], &untouched));
    assert!(Rc::ptr_eq(&first[1], &untouched_other));
    assert!(!Rc::ptr_eq(&second[0], &to_change));
    assert_eq!(*second[0], range(5, 5).join(range(7, 8)));
    assert!(Rc::ptr_eq(&second[1], &changed_other));
}

#[cfg(all(test, feature = "internal-checker"))]
#[test]
fn guarded_output_cut_pairs_match_small_closed_integer_ranges() {
    use BinaryOperator::{Equal, Greater, GreaterEqual, Less, LessEqual, NotEqual};
    let relations = [Equal, Greater, GreaterEqual, Less, LessEqual, NotEqual]
        .map(|op| IntegerRelation::from_operator(&op).unwrap());
    let points: Vec<_> = (-3..=3).map(BigInt::from).collect();
    for left in &points {
        for right in &points {
            for a in relations {
                for b in relations {
                    let query = ForbiddenCutPair::new((a, left), (b, right));
                    for lo in 0..points.len() {
                        for hi in lo..points.len() {
                            let expected = points[lo..=hi].iter().any(|n|
                                a.accepts(n.cmp(left)) && b.accepts(n.cmp(right)));
                            assert_eq!(query.hits(&points[lo], &points[hi]), expected);
                        }
                    }
                }
            }
        }
    }
    let wide = BigInt::from(1u128 << 64);
    let previous = &wide - 1u8;
    assert!(consecutive_integer_cuts(&previous, &wide));
    assert!(consecutive_integer_cuts(&(-&wide), &(-&previous)));
    assert!(!consecutive_integer_cuts(&(&previous - 1u8), &wide));
}

#[cfg(all(test, feature = "internal-checker"))]
#[test]
fn strict_peer_typed_tuple_requires_both_axes_in_one_unchanged_return_row() {
    let binding = |offset| ReferenceBindingId {
        source: wosy_syntax::SourceIdentity::new("local".into(), "demo".into(), "src/main.w".into(), "tuple-row".into()),
        function_span: super::ByteSpan::new(0, 9),
        declaration_span: super::ByteSpan::new(offset, offset + 1),
        block_span: super::ByteSpan::new(4, 9),
        kind: super::ReferenceBindingKind::Declared,
    };
    let first = binding(1);
    let second = binding(2);
    let decoy = binding(3);
    let returned = BigInt::from(7);
    let path = IntegerReturningPath {
        value: IntegerReturn::Const(&returned),
        ty: &ScalarType::I16,
        state: IntegerInputs(vec![
            vec![Rc::new(IntegerSet::singleton(&BigInt::from(1))), Rc::new(IntegerSet::singleton(&BigInt::from(-8)))],
            vec![Rc::new(IntegerSet::singleton(&BigInt::from(2))), Rc::new(IntegerSet::singleton(&BigInt::from(-3)))],
        ]),
        axes: vec![&first, &second],
    };
    fn singleton<'a>(binding: &'a ReferenceBindingId, value: &'a BigInt) -> ImmutableEntryRange<'a> {
        ImmutableEntryRange { binding, lower: value, upper: value }
    }
    let one = BigInt::from(1);
    let minus_eight = BigInt::from(-8);
    let minus_three = BigInt::from(-3);
    assert!(path.covers_immutable_entry_domain(singleton(&first, &one), Some(singleton(&second, &minus_eight))));
    assert!(!path.covers_immutable_entry_domain(singleton(&first, &one), Some(singleton(&second, &minus_three))));
    assert!(!path.covers_immutable_entry_domain(singleton(&first, &one), Some(singleton(&decoy, &minus_eight))));
}

#[cfg(all(test, feature = "internal-checker"))]
#[test]
fn proof_show_output_union_is_lazy_and_exact_for_disconnected_aliases() {
    let binding = ReferenceBindingId {
        source: wosy_syntax::SourceIdentity::new("local".into(), "demo".into(), "src/main.w".into(), "show-union".into()),
        function_span: super::ByteSpan::new(0, 9),
        declaration_span: super::ByteSpan::new(1, 2),
        block_span: super::ByteSpan::new(4, 9),
        kind: super::ReferenceBindingKind::Declared,
    };
    let literal = |value| IntegerSet::singleton(&BigInt::from(value));
    let values = literal(-1).join(literal(1));
    let alias = IntegerReturningPath {
        value: IntegerReturn::Alias(&binding), ty: &ScalarType::I16,
        state: IntegerInputs(vec![vec![Rc::new(values)]]), axes: vec![&binding],
    };
    let relation = |operator| IntegerRelation::from_operator(&operator).unwrap();
    let zero = BigInt::from(0);
    let minus_one = BigInt::from(-1);
    let show = IntegerShowOutput::Disjunction([(relation(BinaryOperator::Less), zero.clone()), (relation(BinaryOperator::Greater), zero.clone())]);
    let mut allowed = None;
    assert_eq!(alias.shows_output_with_bound(relation(BinaryOperator::Less), &zero, Some(&show), None, &mut allowed), Some(false));
    assert!(allowed.is_none());

    let constant = IntegerReturningPath {
        value: IntegerReturn::Const(&minus_one), ty: &ScalarType::I16,
        state: IntegerInputs(vec![Vec::new()]), axes: Vec::new(),
    };
    assert_eq!(constant.shows_output_with_bound(relation(BinaryOperator::Less), &zero, Some(&show), None, &mut allowed), Some(true));
    assert!(allowed.is_none());

    assert_eq!(alias.shows_output_with_bound(relation(BinaryOperator::NotEqual), &zero, Some(&show), None, &mut allowed), Some(true));
    assert!(allowed.is_some());
}
