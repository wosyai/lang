use std::{borrow::Cow, collections::HashMap, rc::Rc};

use num_bigint::BigInt;

use super::{
    checker_directive::CheckerOperation,
    control::{AssignmentReplacement, BindingInitialization, BindingRegion, BlockItemKind, BlockPosition, ConditionalRegion},
    reference::{BlockOrdinal, DirectDestinationIdentity, FunctionCallSites, OccurrenceEdge, OccurrenceNode, OccurrenceOrdinal, OccurrenceParent, OccurrenceStructure, PlainNameIdentity},
    symbolic_integer::IntegerSet,
    typecheck::is_integer_type,
    BinaryOperator, ReferenceBindingId, ScalarExpression, ScalarType, UnaryOperator,
};
use super::{symbolic_integer::IntegerRelation, reference::CheckerIntegerCallArgument};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum CheckerPredicate {
    Boolean(bool),
    Integer(IntegerPredicate),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct IntegerPredicate {
    pub(super) binding: ReferenceBindingId,
    ty: ScalarType,
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
    pub(super) values: IntegerSet,
    pub(super) guards: Vec<ImportedEntryGuard<'a>>,
}

pub(super) struct ImportedEntryGuard<'a> {
    pub(super) slot: usize,
    pub(super) relation: IntegerRelation,
    pub(super) bound: &'a BigInt,
}

type IntegerRow = Vec<Rc<IntegerSet>>;
#[derive(Default)]
struct IntegerRoots<'a> {
    axes: Vec<(&'a ReferenceBindingId, &'a ScalarType)>,
    by_binding: HashMap<&'a ReferenceBindingId, usize>,
    parameters: usize,
    reached_obligation: bool,
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
        self.0.retain_mut(|row| {
            let retained = row[axis].intersection(survivors);
            if retained.is_empty() {
                return false;
            }
            if retained != *row[axis] {
                row[axis] = Rc::new(retained);
            }
            true
        });
    }

    fn split(
        self,
        roots: &IntegerRoots<'_>,
        predicate: &CheckerPredicate,
    ) -> Option<(Self, Self, Option<usize>)> {
        let predicate = match predicate {
            CheckerPredicate::Boolean(true) => return Some((self, Self::default(), None)),
            CheckerPredicate::Boolean(false) => return Some((Self::default(), self, None)),
            CheckerPredicate::Integer(predicate) => predicate,
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

#[derive(Debug, PartialEq, Eq)]
pub(super) enum IntegerReturn<'a> {
    Alias(&'a ReferenceBindingId),
    Const(&'a BigInt),
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
    Conditional {
        guard: CheckerPredicate,
        then_claim: IntegerReturn<'a>,
        else_claim: IntegerReturn<'a>,
    },
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

    fn shows(self, proof: &IntegerShowProof<'_>) -> Option<bool> {
        let (guard, then_claim, else_claim) = match proof {
            IntegerShowProof::Equality(claim) => return shows_return_equality(&self.state, &self.axes, &self.value, claim),
            IntegerShowProof::Conditional { guard, then_claim, else_claim } => (guard, then_claim, else_claim),
        };
        let (yes, no) = match guard {
            CheckerPredicate::Boolean(true) => (self.state, IntegerInputs::default()),
            CheckerPredicate::Boolean(false) => (IntegerInputs::default(), self.state),
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
    run_integer_callable::<false, false>(calls, integer_tail_type(calls), None, None, None, None, None, None)
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
    Projected(Rc<IntegerSet>),
}

pub(super) enum IntegerRequirementContinuation {
    Feasible,
    Empty,
    Projected(Rc<IntegerSet>),
}

struct IntegerFailureCollection<'a, 'sink> {
    continuation: IntegerRequirementContinuation,
    projected: Option<Rc<IntegerSet>>,
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
        IntegerRequirementEntry::Projected(values) => (None, None, Some(values)),
    };
    let mut collected = IntegerFailureCollection {
        continuation: IntegerRequirementContinuation::Empty, projected, allow_projected_calls, sink,
    };
    let ProofCoverage::Applicable(active) = run_integer_callable::<false, true>(
        calls, tail, None, None, None, exact, handle_call, Some(&mut collected),
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

pub(super) enum NestedIntegerArgument<'a> {
    Single(Option<ExactIntegerArgument<'a>>),
    Projected { ty: &'a ScalarType, axis: usize, values: Rc<IntegerSet> },
}

fn actual_nested_argument<'a>(
    calls: &'a FunctionCallSites,
    expression: OccurrenceOrdinal,
    roots: &IntegerRoots<'_>,
    path: &'a IntegerInputs,
    allow_projection: bool,
) -> NestedIntegerArgument<'a> {
    let Some((argument, ty)) = checked_integer_call_argument_at(&calls.structure, expression, 0) else {
        return NestedIntegerArgument::Single(None);
    };
    if let Some(value) = calls.structure.checker_integer_literals.get(&argument) {
        return NestedIntegerArgument::Single(Some((ty, value)));
    }
    let Some((axis, binding_type)) = calls.prepared_plain_name(argument).and_then(|name| match name {
        PlainNameIdentity::Lexical(binding) => roots.lookup(binding),
        _ => None,
    }) else {
        return NestedIntegerArgument::Single(None);
    };
    if binding_type != ty {
        return NestedIntegerArgument::Single(None);
    }
    if let Some(value) = path.singleton_at(axis) {
        return NestedIntegerArgument::Single(Some((ty, value)));
    }
    if allow_projection {
        if let Some(values) = path.project_axis(axis) {
            return NestedIntegerArgument::Projected { ty, axis, values };
        }
    }
    NestedIntegerArgument::Single(None)
}

pub(super) type ActualCallHandler<'a, 'handler> =
    dyn FnMut(
        &'a FunctionCallSites, OccurrenceOrdinal, NestedIntegerArgument<'_>, &ScalarType,
    ) -> Option<ActualCallResult> + 'handler;

pub(super) fn check_integer_requirements_with_calls<'a>(
    calls: &'a FunctionCallSites,
    exact_arguments: Option<&[&BigInt]>,
    handle_call: &mut ActualCallHandler<'a, '_>,
) -> Option<Result<bool, IntegerCheckFailure<'a>>> {
    let tail = if exact_arguments.is_none() { integer_tail_type(calls) } else { None };
    run_integer_callable::<false, false>(calls, tail, None, None, None, exact_arguments, Some(handle_call), None)
        .map(|result| result.map(|coverage| matches!(coverage, ProofCoverage::Applicable(true))))
}

pub(super) fn proves_requirement_with_imported_root_local(
    calls: &FunctionCallSites,
    local: ImportedLocal<'_>,
) -> Option<bool> {
    if local.values.is_empty() {
        return None;
    }
    match run_integer_callable::<true, false>(calls, integer_tail_type(calls), None, None, Some(local), None, None, None)? {
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
    match observe_integer_returns_in_scope(calls, None, None)? {
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
    observe_integer_returns_in_scope(calls, entry_filter, None)
}

#[cfg(all(test, feature = "internal-checker"))]
thread_local! {
    static INTEGER_OBSERVATIONS: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}

#[cfg(all(test, feature = "internal-checker"))]
pub(super) fn take_integer_observations() -> (usize, usize) {
    INTEGER_OBSERVATIONS.with(|observations| observations.replace((0, 0)))
}

fn observe_integer_returns_in_scope<'a>(
    calls: &'a FunctionCallSites,
    entry_filter: Option<&[CheckerPredicate]>,
    exact_arguments: Option<&[&BigInt]>,
) -> Option<Result<ProofCoverage<Vec<IntegerReturningPath<'a>>>, IntegerCheckFailure<'a>>> {
    let tail = integer_tail_type(calls)?;
    #[cfg(all(test, feature = "internal-checker"))]
    INTEGER_OBSERVATIONS.with(|observations| {
        let (arbitrary, actual) = observations.get();
        observations.set(if exact_arguments.is_some() { (arbitrary, actual + 1) } else { (arbitrary + 1, actual) });
    });
    let mut paths = Vec::new();
    match run_integer_callable::<false, false>(calls, Some(tail), entry_filter, Some(&mut paths), None, exact_arguments, None, None)? {
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
    let ProofCoverage::Applicable(paths) = observe_integer_returns_in_scope(calls, None, Some(arguments))?.ok()? else {
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
    let paths = match observe_integer_returns_in_scope(calls, entry_filter, None)? {
        Ok(ProofCoverage::Applicable(paths)) => paths,
        Ok(ProofCoverage::NoApplicableInputs) => return Some(Ok(ProofCoverage::NoApplicableInputs)),
        Err(failure) => return Some(Err(failure)),
    };
    for path in paths {
        if !path.output_satisfies(relation, bound)? {
            return Some(Ok(ProofCoverage::Applicable(false)));
        }
        if let Some(proof) = proof {
            if !path.shows(proof)? {
                return Some(Ok(ProofCoverage::Applicable(false)));
            }
        }
    }
    Some(Ok(ProofCoverage::Applicable(true)))
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

fn run_integer_callable<'a, const USE_IMPORTED_BOUND: bool, const COLLECT: bool>(
    calls: &'a FunctionCallSites,
    tail: Option<&'a ScalarType>,
    entry_filter: Option<&[CheckerPredicate]>,
    returns: Option<&mut Vec<IntegerReturningPath<'a>>>,
    imported_local: Option<ImportedLocal<'a>>,
    exact_arguments: Option<&[&BigInt]>,
    mut handle_call: Option<&mut ActualCallHandler<'a, '_>>,
    mut failures: Option<&mut IntegerFailureCollection<'a, '_>>,
) -> Option<Result<ProofCoverage<bool>, IntegerCheckFailure<'a>>> {
    let boundary = &calls.boundary;
    let checker_parameters = boundary.checker_parameter_bindings.as_deref()?;
    debug_assert_eq!(boundary.parameters.len(), checker_parameters.len());
    let mut roots = IntegerRoots {
        parameters: boundary.parameters.len(),
        imported_local,
        ..IntegerRoots::default()
    };
    for (binding, parameter) in checker_parameters.iter().zip(&boundary.parameters) {
        roots.add(binding, &parameter.ty);
    }
    let projected = if COLLECT { failures.as_deref_mut()?.projected.take() } else { None };
    let projected_entry = projected.is_some();
    let mut row: IntegerRow = if let Some(projected) = projected {
        if boundary.parameters.len() != 1 || projected.is_empty() {
            return None;
        }
        vec![projected]
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
    let result = walk_integer_block::<USE_IMPORTED_BOUND, COLLECT>(calls, &mut roots, calls.structure.root, IntegerInputs(vec![row]), tail, returns, &mut handle_call, &mut failures)?;
    if USE_IMPORTED_BOUND && roots.imported_local.is_some() {
        return None;
    }
    if COLLECT {
        failures.as_deref_mut()?.continuation = match &result {
            IntegerControl::Continue(path) if path.is_unreachable() => IntegerRequirementContinuation::Empty,
            IntegerControl::Continue(path) if projected_entry =>
                IntegerRequirementContinuation::Projected(path.project_axis(0)?),
            IntegerControl::Returned if projected_entry => return None,
            IntegerControl::Continue(_) | IntegerControl::Returned => IntegerRequirementContinuation::Feasible,
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

fn walk_integer_block<'a, const USE_IMPORTED_BOUND: bool, const COLLECT: bool>(
    calls: &'a FunctionCallSites,
    roots: &mut IntegerRoots<'a>,
    block: BlockOrdinal,
    mut path: IntegerInputs,
    tail: Option<&'a ScalarType>,
    mut returns: Option<&mut Vec<IntegerReturningPath<'a>>>,
    handle_call: &mut Option<&mut ActualCallHandler<'a, '_>>,
    failures: &mut Option<&mut IntegerFailureCollection<'a, '_>>,
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
                    let project = COLLECT && failures.as_ref().is_some_and(|collection| collection.allow_projected_calls);
                    let argument = actual_nested_argument(calls, expression, roots, &path, project);
                    let projected = match &argument {
                        NestedIntegerArgument::Projected { axis, values, .. } => Some((*axis, Rc::clone(values))),
                        _ => None,
                    };
                    match handle(calls, expression, argument, &ScalarType::Unit)? {
                        ActualCallResult::Unit => {}
                        ActualCallResult::EmptyUnit if COLLECT => path = IntegerInputs::default(),
                        ActualCallResult::ProjectedUnit(survivors) if COLLECT => {
                            let (axis, sent) = projected?;
                            if !Rc::ptr_eq(&sent, &survivors) {
                                path.restrict_axis(axis, &survivors);
                            }
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
                            walk_integer_block::<USE_IMPORTED_BOUND, COLLECT>(calls, roots, arm, incoming, None, None, handle_call, failures)?
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
                        match walk_integer_block::<USE_IMPORTED_BOUND, COLLECT>(calls, roots, arm, incoming, Some(expected), returns.as_deref_mut(), handle_call, failures)? {
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
                } else {
                    let PlainNameIdentity::Lexical(binding) = calls.prepared_plain_name(slot.expression)?
                    else {
                        return None;
                    };
                    let (_, binding_type) = roots.lookup(binding)?;
                    if binding_type != ty {
                        return None;
                    }
                    IntegerReturn::Alias(binding)
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
                    roots.add(&origin.binding, &receiver.ty);
                    path.append_local(local.values);
                    continue;
                }
                if let Some(origin) = structure.checker_integer_call_locals.get(&(block, item.position)) {
                    let handle = handle_call.as_mut()?;
                    let [receiver] = binding.receivers() else {
                        return None;
                    };
                    let argument = actual_nested_argument(calls, origin.call, roots, &path, false);
                    let ActualCallResult::Integer(value) = handle(calls, origin.call, argument, &receiver.ty)? else {
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
