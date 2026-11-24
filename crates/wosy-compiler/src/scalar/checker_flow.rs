use std::{borrow::Cow, collections::HashMap, rc::Rc};

use num_bigint::BigInt;

use super::{
    checker_directive::CheckerOperation,
    control::{AssignmentReplacement, BindingInitialization, BindingRegion, BlockItemKind, BlockPosition, CheckedCallSignature, ConditionalRegion, FinalOutputSlot},
    reference::{BlockOrdinal, DirectDestinationIdentity, FunctionCallSites, OccurrenceEdge, OccurrenceNode, OccurrenceOrdinal, OccurrenceParent, OccurrenceStructure, PlainNameIdentity},
    symbolic_integer::{IntegerCutUnion, IntegerIntersection, IntegerSet},
    typecheck::is_integer_type,
    BinaryOperator, ReferenceBindingId, ScalarExpression, ScalarType, UnaryOperator,
};
use super::{symbolic_integer::IntegerRelation, reference::CheckerIntegerCallArgument};

#[cfg(feature = "internal-checker")]
use super::symbolic_integer::{consecutive_integer_cuts, IntegerShowConstraints};

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
    Shared {
        values: &'a std::cell::OnceCell<Rc<IntegerSet>>,
        bound: &'a BigInt,
        guards: &'a [ImportedEntryGuard<'a>],
        domain: Option<(usize, &'a IntegerSet)>,
    },
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

#[cfg(feature = "internal-checker")]
struct RootBranchSlot {
    block: BlockOrdinal,
    item: usize,
    parent: OccurrenceOrdinal,
    condition: OccurrenceOrdinal,
    output: Option<usize>,
}

trait ScopedAuthoredReturn<'a>: Copy {
    const ENABLED: bool;
    #[cfg(feature = "internal-checker")]
    const ALLOW_ORDINARY_INTEGER_LOCALS: bool = false;
    fn at(
        self, calls: &FunctionCallSites, block: BlockOrdinal, slot: FinalOutputSlot, ty: &ScalarType,
        roots: &IntegerRoots<'_>, path: &IntegerInputs,
    ) -> Option<&'a BigInt>;

    #[cfg(feature = "internal-checker")]
    fn prefer_legacy_root_compound(
        self, _: &FunctionCallSites, _: BlockOrdinal, _: OccurrenceOrdinal, _: &IntegerRoots<'_>,
    ) -> bool {
        false
    }

    #[cfg(feature = "internal-checker")]
    fn split_unprepared(
        self, _: &FunctionCallSites, _: RootBranchSlot, _: &IntegerRoots<'_>, _: IntegerInputs,
    ) -> Option<(IntegerInputs, IntegerInputs, Option<usize>)> {
        None
    }

    #[cfg(feature = "internal-checker")]
    const OBSERVE_RETURN: bool = false;

    #[cfg(feature = "internal-checker")]
    const REQUIRE_TOTAL_RETURN: bool = false;

    #[cfg(feature = "internal-checker")]
    fn observe_return(
        self, _: FinalOutputSlot, _: IntegerReturn<'a>, _: &IntegerRoots<'a>, _: &IntegerInputs,
    ) -> Option<()> {
        Some(())
    }

    fn branch(
        self, _: &OccurrenceStructure, _: BlockOrdinal, _: FinalOutputSlot, _: usize, _: BlockOrdinal,
    ) -> Self {
        self
    }
}

impl<'a> ScopedAuthoredReturn<'a> for () {
    const ENABLED: bool = false;

    fn at(
        self, _: &FunctionCallSites, _: BlockOrdinal, _: FinalOutputSlot, _: &ScalarType,
        _: &IntegerRoots<'_>, _: &IntegerInputs,
    ) -> Option<&'a BigInt> {
        None
    }
}

#[cfg(feature = "internal-checker")]
#[derive(Clone, Copy)]
struct OrderedRootConditions<'a, const EXACT_COMPOUND: bool = false, const ORDINARY_LOCALS: bool = false>(&'a super::ScalarBlock);

#[cfg(feature = "internal-checker")]
impl<'a, const EXACT_COMPOUND: bool, const ORDINARY_LOCALS: bool> ScopedAuthoredReturn<'a>
    for OrderedRootConditions<'a, EXACT_COMPOUND, ORDINARY_LOCALS>
{
    const ENABLED: bool = false;
    const ALLOW_ORDINARY_INTEGER_LOCALS: bool = ORDINARY_LOCALS;

    fn at(
        self, _: &FunctionCallSites, _: BlockOrdinal, _: FinalOutputSlot, _: &ScalarType,
        _: &IntegerRoots<'_>, _: &IntegerInputs,
    ) -> Option<&'a BigInt> {
        None
    }

    fn prefer_legacy_root_compound(
        self, calls: &FunctionCallSites, block: BlockOrdinal, condition: OccurrenceOrdinal, roots: &IntegerRoots<'_>,
    ) -> bool {
        // A root-level pair of formal/literal cuts was already supported.
        // Keep the existing compact union path, notably for repeated ORs
        // that cover the whole type without creating any alternative rows.
        if !ORDINARY_LOCALS || !EXACT_COMPOUND || block != calls.structure.root || !calls.assignments.is_empty() {
            return false;
        }
        use super::reference::CheckerOrdinaryBoolean as Boolean;
        let Some(Boolean::Or(operands) | Boolean::And(operands)) =
            calls.structure.checker_ordinary_booleans.get(&condition) else { return false; };
        let Some(formals) = calls.boundary.checker_parameter_bindings.as_deref() else { return false; };
        operands.iter().all(|operand| {
            let Boolean::Integer(predicate) = operand else { return false; };
            roots.lookup(&predicate.binding)
                .is_some_and(|(axis, _)| formals.get(axis) == Some(&predicate.binding))
        })
    }

    fn split_unprepared(
        self, calls: &FunctionCallSites, slot: RootBranchSlot, roots: &IntegerRoots<'_>, path: IntegerInputs,
    ) -> Option<(IntegerInputs, IntegerInputs, Option<usize>)> {
        let structure = &calls.structure;
        if slot.block != structure.root || structure.blocks.get(slot.block.0)?.span != self.0.span
            || !calls.assignments.is_empty()
        {
            return None;
        }
        let super::ScalarBlockItem::Expression(branch) = self.0.items.get(slot.item)? else { return None; };
        let edge = match slot.output {
            Some(output) => OccurrenceEdge::FinalOutput { item: slot.item, output },
            None => OccurrenceEdge::ExpressionItem { item: slot.item },
        };
        if structure.edges.get(&(OccurrenceParent::Block(slot.block), edge))
            != Some(&OccurrenceNode::Expression(slot.parent))
        {
            return None;
        }
        let child = |parent, edge| match structure.edges.get(&(OccurrenceParent::Expression(parent), edge)) {
            Some(OccurrenceNode::Expression(ordinal)) => Some(*ordinal),
            _ => None,
        };
        let condition = match branch {
            ScalarExpression::If { condition, .. } | ScalarExpression::UnitIf { condition, .. } => condition,
            _ => return None,
        };
        let ScalarExpression::Binary { operator, left, right, .. } = condition.as_ref() else {
            return None;
        };
        let conjunction = match operator {
            BinaryOperator::Or => false,
            BinaryOperator::And if EXACT_COMPOUND => true,
            _ => return None,
        };
        if child(slot.parent, OccurrenceEdge::Condition) != Some(slot.condition)
            || structure.checked_types.expressions.get(&slot.condition) != Some(&ScalarType::Bool)
        {
            return None;
        }
        let atom = |expression, edge| {
            let ordinal = child(slot.condition, edge)?;
            if structure.checked_types.expressions.get(&ordinal) != Some(&ScalarType::Bool) {
                return None;
            }
            let comparison = IntegerComparison::from_expression(expression)?;
            let left = child(ordinal, OccurrenceEdge::BinaryLeft)?;
            let right = child(ordinal, OccurrenceEdge::BinaryRight)?;
            let PlainNameIdentity::Lexical(binding) = calls.prepared_plain_name(left)? else { return None; };
            let (axis, ty) = roots.lookup(binding)?;
            if calls.boundary.checker_parameter_bindings.as_deref()?.get(axis) != Some(binding)
                || !is_integer_type(ty)
                || structure.checked_types.expressions.get(&left) != Some(ty)
                || structure.checked_types.expressions.get(&right) != Some(ty)
            {
                return None;
            }
            Some((axis, ty, comparison.operator, comparison.bound))
        };
        let (axis, ty, left_operator, left_bound) = atom(left.as_ref(), OccurrenceEdge::BinaryLeft)?;
        let (other_axis, other_ty, right_operator, right_bound) = atom(right.as_ref(), OccurrenceEdge::BinaryRight)?;
        if !conjunction && axis != other_axis && !EXACT_COMPOUND {
            return None;
        }
        let (min, max) = super::typecheck::integer_bounds(ty)?;
        let other_bounds = if ty == other_ty { None } else { Some(super::typecheck::integer_bounds(other_ty)?) };
        let (other_min, other_max) = other_bounds.as_ref().map_or((&min, &max), |(lo, hi)| (lo, hi));
        if left_bound.as_ref() < &min || left_bound.as_ref() > &max
            || right_bound.as_ref() < other_min || right_bound.as_ref() > other_max
        {
            return None;
        }
        if conjunction {
            let (left_true, left_false) = path.split_axis(axis, left_operator, &left_bound)?;
            let (yes, right_false) = left_true.split_axis(other_axis, right_operator, &right_bound)?;
            return Some((yes, left_false.join(right_false, Some(axis)), Some(axis)));
        }
        if axis != other_axis {
            let (left_true, left_false) = path.split_axis(axis, left_operator, &left_bound)?;
            let (right_true, no) = left_false.split_axis(other_axis, right_operator, &right_bound)?;
            return Some((left_true.join(right_true, Some(axis)), no, Some(axis)));
        }
        let accepted = IntegerCutUnion::new(
            (IntegerRelation::from_operator(left_operator)?, &left_bound),
            (IntegerRelation::from_operator(right_operator)?, &right_bound), (&min, &max),
        );
        let (yes, no) = path.split_partition(axis, &accepted);
        Some((yes, no, Some(axis)))
    }
}

#[cfg(feature = "internal-checker")]
pub(super) struct SymbolicLiteralFailure<'a> {
    pub(super) output: super::ByteSpan,
    pub(super) returned: BigInt,
    pub(super) state: IntegerInputs,
    pub(super) axes: Vec<(&'a ReferenceBindingId, &'a ScalarType)>,
}

#[cfg(feature = "internal-checker")]
impl<'a> SymbolicLiteralFailure<'a> {
    fn from_row(
        slot: FinalOutputSlot, roots: &IntegerRoots<'a>, row: &IntegerRow, axis: Option<usize>,
        returned: Cow<'_, BigInt>, entry: Option<(usize, &BigInt)>,
    ) -> Self {
        let mut state = row[..roots.parameters].to_vec();
        if let Some((axis, value)) = entry {
            state[axis] = Rc::new(IntegerSet::singleton(value));
        }
        if let Some(axis) = axis {
            state[axis] = Rc::new(IntegerSet::singleton(returned.as_ref()));
        }
        Self { output: slot.span, returned: returned.into_owned(), state: IntegerInputs::from_cells(vec![state]),
            axes: roots.axes[..roots.parameters].to_vec() }
    }
}

#[cfg(feature = "internal-checker")]
fn symbolic_return_axis(value: IntegerReturn<'_>, roots: &IntegerRoots<'_>) -> Option<Option<usize>> {
    match value {
        IntegerReturn::Const(_) => Some(None),
        IntegerReturn::Alias(binding) => {
            let (axis, _) = roots.lookup(binding)?;
            (axis < roots.parameters).then_some(Some(axis))
        }
    }
}

#[cfg(feature = "internal-checker")]
pub(super) struct SymbolicLiteralCheck<'a, 'body> {
    pub(super) relation: IntegerRelation,
    pub(super) bound: &'a BigInt,
    pub(super) failure: Option<SymbolicLiteralFailure<'body>>,
}

#[cfg(feature = "internal-checker")]
struct BatchedLiteralEvidence<'a, 'body, 'observation> {
    returned: bool,
    pending: usize,
    checks: &'observation mut [SymbolicLiteralCheck<'a, 'body>],
}

#[cfg(feature = "internal-checker")]
#[derive(Clone, Copy)]
struct OrderedRootLiteralProofs<'a, 'bound, 'checks, 'observation, const STEP_FIRST: bool> {
    conditions: OrderedRootConditions<'a>,
    observed: &'observation std::cell::RefCell<BatchedLiteralEvidence<'bound, 'a, 'checks>>,
}

#[cfg(feature = "internal-checker")]
impl<'a, 'bound, const STEP_FIRST: bool> ScopedAuthoredReturn<'a> for OrderedRootLiteralProofs<'a, 'bound, '_, '_, STEP_FIRST> {
    const ENABLED: bool = false;
    const OBSERVE_RETURN: bool = true;

    fn at(
        self, _: &FunctionCallSites, _: BlockOrdinal, _: FinalOutputSlot, _: &ScalarType,
        _: &IntegerRoots<'_>, _: &IntegerInputs,
    ) -> Option<&'a BigInt> {
        None
    }

    fn split_unprepared(
        self, calls: &FunctionCallSites, slot: RootBranchSlot, roots: &IntegerRoots<'_>, path: IntegerInputs,
    ) -> Option<(IntegerInputs, IntegerInputs, Option<usize>)> {
        self.conditions.split_unprepared(calls, slot, roots, path)
    }

    fn observe_return(
        self, slot: FinalOutputSlot, value: IntegerReturn<'a>, roots: &IntegerRoots<'a>, path: &IntegerInputs,
    ) -> Option<()> {
        let axis = symbolic_return_axis(value, roots)?;
        let mut observed = self.observed.borrow_mut();
        observed.returned = true;
        let BatchedLiteralEvidence { pending, checks, .. } = &mut *observed;
        if *pending == 0 {
            return Some(());
        }
        for row in &path.0 {
            let (domain, low, high) = match value {
                IntegerReturn::Const(value) => (None, value, value),
                IntegerReturn::Alias(_) => {
                    let domain = row.get(axis?)?;
                    let (low, high) = domain.extremes()?;
                    (Some(domain), low, high)
                }
            };
            let rejected_point = |relation: IntegerRelation, bound: &'bound BigInt| {
                let rejected = relation.complement();
                if rejected.accepts(low.cmp(bound)) {
                    Some(low)
                } else if rejected.accepts(high.cmp(bound)) {
                    Some(high)
                } else if rejected.accepts(std::cmp::Ordering::Equal) && domain.is_some_and(|domain|
                    low < bound && bound < high && domain.covers_closed(bound, bound))
                {
                    Some(bound)
                } else {
                    None
                }
            };
            let checks = if STEP_FIRST {
                let (step, goals) = checks.split_first_mut()?;
                if let Some(witness) = rejected_point(step.relation, step.bound) {
                    step.failure = Some(SymbolicLiteralFailure::from_row(slot, roots, row, axis, Cow::Borrowed(witness), None));
                    *pending = 0;
                    return Some(());
                }
                goals
            } else {
                &mut **checks
            };
            for check in checks.iter_mut().filter(|check| check.failure.is_none()) {
                if let Some(witness) = rejected_point(check.relation, check.bound) {
                    check.failure = Some(SymbolicLiteralFailure::from_row(slot, roots, row, axis, Cow::Borrowed(witness), None));
                    *pending -= 1;
                }
            }
            if *pending == 0 || axis.is_none() {
                break;
            }
        }
        Some(())
    }
}

#[cfg(feature = "internal-checker")]
pub(super) enum SymbolicProofPoint<'a> {
    Lower,
    Upper,
    Bound(&'a BigInt),
}

#[cfg(feature = "internal-checker")]
type SymbolicProofSelect<'a> = dyn FnMut(&BigInt, &BigInt, Option<&IntegerSet>) -> Option<SymbolicProofPoint<'a>> + 'a;

#[cfg(feature = "internal-checker")]
struct MultipleProofEvidence<'a, 'check, 'bound> {
    select_witness: &'check mut SymbolicProofSelect<'bound>,
    returned: bool,
    failure: Option<SymbolicLiteralFailure<'a>>,
}

#[cfg(feature = "internal-checker")]
#[derive(Clone, Copy)]
struct OrderedRootMultipleProof<'a, 'check, 'observation, 'bound> {
    conditions: OrderedRootConditions<'a>,
    observed: &'observation std::cell::RefCell<MultipleProofEvidence<'a, 'check, 'bound>>,
}

#[cfg(feature = "internal-checker")]
impl<'a> ScopedAuthoredReturn<'a> for OrderedRootMultipleProof<'a, '_, '_, '_> {
    const ENABLED: bool = false;
    const OBSERVE_RETURN: bool = true;

    fn at(
        self, _: &FunctionCallSites, _: BlockOrdinal, _: FinalOutputSlot, _: &ScalarType,
        _: &IntegerRoots<'_>, _: &IntegerInputs,
    ) -> Option<&'a BigInt> {
        None
    }

    fn split_unprepared(
        self, calls: &FunctionCallSites, slot: RootBranchSlot, roots: &IntegerRoots<'_>, path: IntegerInputs,
    ) -> Option<(IntegerInputs, IntegerInputs, Option<usize>)> {
        self.conditions.split_unprepared(calls, slot, roots, path)
    }

    fn observe_return(
        self, slot: FinalOutputSlot, value: IntegerReturn<'a>, roots: &IntegerRoots<'a>, path: &IntegerInputs,
    ) -> Option<()> {
        let axis = symbolic_return_axis(value, roots)?;
        let mut observed = self.observed.borrow_mut();
        let MultipleProofEvidence { select_witness, returned, failure } = &mut *observed;
        for row in &path.0 {
            *returned = true;
            let (lo, hi, domain) = match value {
                IntegerReturn::Const(value) => (value, value, None),
                IntegerReturn::Alias(_) => {
                    let domain = row.get(axis?)?;
                    let (lo, hi) = domain.extremes()?;
                    (lo, hi, Some(domain.as_ref()))
                }
            };
            if let Some(point) = select_witness(lo, hi, domain) {
                let point = match point {
                    SymbolicProofPoint::Lower => lo,
                    SymbolicProofPoint::Upper => hi,
                    SymbolicProofPoint::Bound(point) => point,
                };
                *failure = Some(SymbolicLiteralFailure::from_row(slot, roots, row, axis, Cow::Borrowed(point), None));
            }
            if axis.is_none() {
                break;
            }
        }
        Some(())
    }
}

#[cfg(feature = "internal-checker")]
struct LiteralGuardAtReturn<'a> {
    axis: usize,
    relation: IntegerRelation,
    bound: &'a BigInt,
    nonfailure: Option<IntegerCutUnion<'a>>,
}

#[cfg(feature = "internal-checker")]
#[derive(Clone, Copy)]
struct OrderedRootLiteralProof<'a, 'observation, const TOTAL: bool> {
    conditions: OrderedRootConditions<'a, TOTAL>,
    relation: IntegerRelation,
    bound: &'observation BigInt,
    guard: Option<&'observation LiteralGuardAtReturn<'observation>>,
    observed: &'observation std::cell::RefCell<(bool, Option<SymbolicLiteralFailure<'a>>)>,
}

#[cfg(feature = "internal-checker")]
impl<'a, const TOTAL: bool> ScopedAuthoredReturn<'a> for OrderedRootLiteralProof<'a, '_, TOTAL> {
    const ENABLED: bool = false;
    const OBSERVE_RETURN: bool = true;
    const REQUIRE_TOTAL_RETURN: bool = TOTAL;

    fn at(
        self, _: &FunctionCallSites, _: BlockOrdinal, _: FinalOutputSlot, _: &ScalarType,
        _: &IntegerRoots<'_>, _: &IntegerInputs,
    ) -> Option<&'a BigInt> {
        None
    }

    fn split_unprepared(
        self, calls: &FunctionCallSites, slot: RootBranchSlot, roots: &IntegerRoots<'_>, path: IntegerInputs,
    ) -> Option<(IntegerInputs, IntegerInputs, Option<usize>)> {
        self.conditions.split_unprepared(calls, slot, roots, path)
    }

    fn observe_return(
        self, slot: FinalOutputSlot, value: IntegerReturn<'a>, roots: &IntegerRoots<'a>, path: &IntegerInputs,
    ) -> Option<()> {
        let axis = symbolic_return_axis(value, roots)?;
        let mut observed = self.observed.borrow_mut();
        if observed.1.is_some() {
            return Some(());
        }
        let rejected = self.relation.complement();
        let mut applicable = self.guard.is_none();
        let mut failing = None;
        for row in &path.0 {
            if let Some(guard) = self.guard.filter(|guard| Some(guard.axis) == axis) {
                let (admitted, value) = row.get(guard.axis)?
                    .first_guarded_failure((guard.relation, guard.bound), guard.nonfailure.as_ref()?);
                applicable |= admitted;
                if let Some(value) = value {
                    failing = Some((row, Cow::Owned(value), None));
                    break;
                }
                continue;
            }
            let entry = if let Some(guard) = self.guard {
                let Some(point) = row.get(guard.axis)?.first_comparison_point(guard.relation, guard.bound) else {
                    continue;
                };
                applicable = true;
                Some((guard.axis, point))
            } else {
                None
            };
            let returned = match (value, axis) {
                (IntegerReturn::Const(value), _) => rejected.accepts(value.cmp(self.bound)).then_some(value),
                (IntegerReturn::Alias(_), Some(axis)) => row.get(axis)?.first_comparison_point(rejected, self.bound),
                _ => return None,
            };
            if let Some(returned) = returned {
                failing = Some((row, Cow::Borrowed(returned), entry));
                break;
            }
            if matches!(value, IntegerReturn::Const(_)) {
                break;
            }
        }
        observed.0 |= applicable;
        if let Some((row, returned, entry)) = failing {
            observed.1 = Some(SymbolicLiteralFailure::from_row(slot, roots, row, axis, returned, entry));
        }
        Some(())
    }
}

#[cfg(feature = "internal-checker")]
#[derive(Clone, Copy)]
struct OrderedRootEntryEquality<'a, 'scope> {
    conditions: OrderedRootConditions<'a>,
    old: usize,
    guard: Option<(usize, IntegerRelation, &'scope BigInt)>,
    observed: &'scope std::cell::RefCell<(bool, Option<SymbolicLiteralFailure<'a>>)>,
}

#[cfg(feature = "internal-checker")]
impl<'a> ScopedAuthoredReturn<'a> for OrderedRootEntryEquality<'a, '_> {
    const ENABLED: bool = false;
    const OBSERVE_RETURN: bool = true;

    fn at(
        self, _: &FunctionCallSites, _: BlockOrdinal, _: FinalOutputSlot, _: &ScalarType,
        _: &IntegerRoots<'_>, _: &IntegerInputs,
    ) -> Option<&'a BigInt> {
        None
    }

    fn split_unprepared(
        self, calls: &FunctionCallSites, slot: RootBranchSlot, roots: &IntegerRoots<'_>, path: IntegerInputs,
    ) -> Option<(IntegerInputs, IntegerInputs, Option<usize>)> {
        self.conditions.split_unprepared(calls, slot, roots, path)
    }

    fn observe_return(
        self, slot: FinalOutputSlot, value: IntegerReturn<'a>, roots: &IntegerRoots<'a>, path: &IntegerInputs,
    ) -> Option<()> {
        let returned_axis = symbolic_return_axis(value, roots)?;
        let mut observed = self.observed.borrow_mut();
        if observed.1.is_some() {
            return Some(());
        }
        for row in &path.0 {
            let restricted = self.guard.map(|(axis, relation, bound)| {
                let original = row[axis].as_ref();
                let admitted = if original.every(relation, bound) {
                    Cow::Borrowed(original)
                } else {
                    Cow::Owned(original.keep_relation(relation, bound))
                };
                (axis, admitted)
            });
            if restricted.as_ref().is_some_and(|(_, axis)| axis.is_empty()) {
                continue;
            }
            observed.0 = true;
            if returned_axis == Some(self.old) {
                break;
            }
            let axis = |index| {
                restricted.as_ref().filter(|(slot, _)| index == *slot)
                    .map_or_else(|| row.get(index).map(Rc::as_ref), |(_, admitted)| Some(admitted.as_ref()))
            };
            let (old_min, old_max) = axis(self.old)?.extremes()?;
            let mismatch = match (value, returned_axis) {
                (IntegerReturn::Const(returned), None) => {
                    if returned != old_min { Some((returned, old_min)) }
                    else if returned != old_max { Some((returned, old_max)) }
                    else { None }
                }
                (IntegerReturn::Alias(_), Some(axis_index)) => {
                    let (returned_min, returned_max) = axis(axis_index)?.extremes()?;
                    if returned_min != old_max { Some((returned_min, old_max)) }
                    else if returned_max != old_min { Some((returned_max, old_min)) }
                    else { None }
                }
                _ => return None,
            };
            if let Some((returned, old)) = mismatch {
                let mut failure = SymbolicLiteralFailure::from_row(
                    slot, roots, row, returned_axis, Cow::Borrowed(returned), Some((self.old, old)),
                );
                if let Some((axis, admitted)) = &restricted {
                    if *axis != self.old && Some(*axis) != returned_axis {
                        failure.state.0[0][*axis] = Rc::new(IntegerSet::singleton(admitted.extremes()?.0));
                    }
                }
                observed.1 = Some(failure);
                break;
            }
        }
        Some(())
    }
}

#[cfg(feature = "internal-checker")]
#[derive(Clone, Copy)]
struct NamedAuthoredReturn<'a> {
    block: BlockOrdinal,
    body: &'a super::ScalarBlock,
    value: &'a std::cell::OnceCell<Option<BigInt>>,
    branches: Option<&'a [std::cell::OnceCell<Option<BigInt>>; 2]>,
}

#[cfg(feature = "internal-checker")]
impl<'a> NamedAuthoredReturn<'a> {
    fn output(
        self, structure: &OccurrenceStructure, block: BlockOrdinal, slot: FinalOutputSlot,
    ) -> Option<&'a ScalarExpression> {
        if block != self.block || slot.output != 0 || structure.blocks.get(block.0)?.span != self.body.span
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
        Some(&output.value)
    }

    fn checked_branch(
        self, structure: &OccurrenceStructure, block: BlockOrdinal, slot: FinalOutputSlot,
        index: usize, child: BlockOrdinal,
    ) -> Option<Self> {
        let value = self.branches?.get(index)?;
        let ScalarExpression::If { then_branch, else_branch, .. } = self.output(structure, block, slot)? else {
            return None;
        };
        let (edge, body) = if index == 0 {
            (OccurrenceEdge::ThenBranch, then_branch)
        } else {
            (OccurrenceEdge::ElseBranch, else_branch)
        };
        if structure.edges.get(&(OccurrenceParent::Expression(slot.expression), edge))
            != Some(&OccurrenceNode::Block(child))
            || structure.blocks.get(child.0)?.span != body.span
        {
            return None;
        }
        Some(Self { block: child, body, value, branches: None })
    }
}

#[cfg(feature = "internal-checker")]
impl<'a> ScopedAuthoredReturn<'a> for NamedAuthoredReturn<'a> {
    const ENABLED: bool = true;

    fn branch(
        self, structure: &OccurrenceStructure, block: BlockOrdinal, slot: FinalOutputSlot,
        index: usize, child: BlockOrdinal,
    ) -> Self {
        self.checked_branch(structure, block, slot, index, child).unwrap_or(self)
    }

    fn at(
        self, calls: &FunctionCallSites, block: BlockOrdinal, slot: FinalOutputSlot, ty: &ScalarType,
        roots: &IntegerRoots<'_>, path: &IntegerInputs,
    ) -> Option<&'a BigInt> {
        let structure = &calls.structure;
        let output = self.output(structure, block, slot)?;
        self.value.get_or_init(|| {
            if let ScalarExpression::Unary { operator: UnaryOperator::Negate, operand, .. } = output {
                if let ScalarExpression::Integer { value, .. } = operand.as_ref() {
                    let Some(OccurrenceNode::Expression(magnitude)) = structure.edges.get(&(
                        OccurrenceParent::Expression(slot.expression), OccurrenceEdge::UnaryOperand,
                    )) else { return None; };
                    if structure.checked_types.expressions.get(&slot.expression) != Some(ty)
                        || structure.folded_expressions.get(magnitude)
                            != Some(&super::control::FoldedOccurrence::SignedIntegerMagnitude)
                    {
                        return None;
                    }
                    #[cfg(test)]
                    AUTHORED_RETURN_CALCULATIONS.with(|count| count.set(count.get() + 1));
                    return Some(-value);
                }
            }
            let evaluate_closed = || {
                let (min, max) = super::typecheck::integer_bounds(ty)?;
                #[cfg(test)]
                AUTHORED_RETURN_CALCULATIONS.with(|count| count.set(count.get() + 1));
                nested_authored_integer_return(output, slot.expression, structure, ty, &min, &max)
            };
            let ScalarExpression::Binary { operator, left, right, .. } = output else {
                return is_authored_composite(output).then(evaluate_closed).flatten();
            };
            let operation = AuthoredIntegerOperation::checked(operator)?;
            if is_authored_composite(left) || is_authored_composite(right) {
                return evaluate_closed();
            }
            let parent = OccurrenceParent::Expression(slot.expression);
            let typed_operand = |edge| {
                let Some(OccurrenceNode::Expression(ordinal)) = structure.edges.get(&(parent, edge)) else { return None; };
                (structure.checked_types.expressions.get(ordinal) == Some(ty)).then_some(*ordinal)
            };
            if structure.checked_types.expressions.get(&slot.expression) != Some(ty) {
                return None;
            }
            let left_ordinal = typed_operand(OccurrenceEdge::BinaryLeft)?;
            let right_ordinal = typed_operand(OccurrenceEdge::BinaryRight)?;
            let formal_axis = |ordinal| {
                let Some(PlainNameIdentity::Lexical(binding)) = calls.prepared_plain_name(ordinal) else {
                    return None;
                };
                let (axis, axis_type) = roots.lookup(binding)?;
                (axis_type == ty && calls.boundary.checker_parameter_bindings.as_deref()?.get(axis) == Some(binding))
                    .then_some(axis)
            };
            let exact_formal = |ordinal, literal: &ScalarExpression| {
                if !matches!(literal, ScalarExpression::Integer { .. })
                    && !matches!(literal, ScalarExpression::Unary { operator: UnaryOperator::Negate, operand, .. }
                        if matches!(operand.as_ref(), ScalarExpression::Integer { .. }))
                {
                    return None;
                }
                path.singleton_at(formal_axis(ordinal)?).map(Cow::Borrowed)
            };
            let (left, right) = match (left.as_ref(), right.as_ref(), operator) {
                (ScalarExpression::Name { .. }, ScalarExpression::Name { .. }, BinaryOperator::Add | BinaryOperator::Subtract) => {
                    let left_axis = formal_axis(left_ordinal)?;
                    let right_axis = formal_axis(right_ordinal)?;
                    if left_axis == right_axis {
                        if !matches!(operator, BinaryOperator::Subtract) {
                            return None;
                        }
                        #[cfg(test)]
                        AUTHORED_RETURN_CALCULATIONS.with(|count| count.set(count.get() + 1));
                        return Some(BigInt::from(0));
                    }
                    let (left, right) = path.singleton_pair_at(left_axis, right_axis)?;
                    (Cow::Borrowed(left), Cow::Borrowed(right))
                }
                (ScalarExpression::Name { .. }, literal, BinaryOperator::Add | BinaryOperator::Subtract) =>
                    (exact_formal(left_ordinal, literal)?, concrete_integer_literal(literal)?),
                (literal, ScalarExpression::Name { .. }, BinaryOperator::Add) => {
                    let value = exact_formal(right_ordinal, literal)?;
                    (concrete_integer_literal(literal)?, value)
                }
                (left, right, _) => (concrete_integer_literal(left)?, concrete_integer_literal(right)?),
            };
            let (min, max) = super::typecheck::integer_bounds(ty)?;
            let fits = |value: &BigInt| value >= &min && value <= &max;
            if !fits(&left) || !fits(&right) {
                return None;
            }
            #[cfg(all(test, feature = "internal-checker"))]
            AUTHORED_RETURN_CALCULATIONS.with(|count| count.set(count.get() + 1));
            let value = operation.apply(left, right, &min)?;
            fits(&value).then_some(value)
        }).as_ref()
    }
}

#[cfg(feature = "internal-checker")]
#[derive(Clone, Copy)]
enum AuthoredIntegerOperation {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
}

#[cfg(feature = "internal-checker")]
impl AuthoredIntegerOperation {
    fn checked(operator: &BinaryOperator) -> Option<Self> {
        Some(match operator {
            BinaryOperator::Add => Self::Add,
            BinaryOperator::Subtract => Self::Subtract,
            BinaryOperator::Multiply => Self::Multiply,
            BinaryOperator::Divide => Self::Divide,
            BinaryOperator::Remainder => Self::Remainder,
            _ => return None,
        })
    }

    fn apply(self, left: Cow<'_, BigInt>, right: Cow<'_, BigInt>, min: &BigInt) -> Option<BigInt> {
        Some(match (self, left, right) {
            (Self::Add, Cow::Owned(left), Cow::Owned(right)) => left + right,
            (Self::Add, Cow::Owned(left), right) => left + right.as_ref(),
            (Self::Add, left, Cow::Owned(right)) => right + left.as_ref(),
            (Self::Add, left, right) => left.as_ref() + right.as_ref(),
            (Self::Subtract, Cow::Owned(left), Cow::Owned(right)) => left - right,
            (Self::Subtract, Cow::Owned(left), right) => left - right.as_ref(),
            (Self::Subtract, left, Cow::Owned(right)) => left.as_ref() - right,
            (Self::Subtract, left, right) => left.as_ref() - right.as_ref(),
            (Self::Multiply, left, right) => left.as_ref() * right.as_ref(),
            (operation @ (Self::Divide | Self::Remainder), left, right) => {
                use num_bigint::Sign::{Minus, NoSign};
                let (left, right) = (left.as_ref(), right.as_ref());
                if right.sign() == NoSign || (right.sign() == Minus && right.bits() == 1 && left == min) {
                    return None;
                }
                match operation {
                    Self::Divide => left / right,
                    _ => left % right,
                }
            }
        })
    }
}

#[cfg(feature = "internal-checker")]
fn is_authored_composite(expression: &ScalarExpression) -> bool {
    match expression {
        ScalarExpression::Binary { .. } => true,
        ScalarExpression::Unary { operator: UnaryOperator::Negate, operand, .. } =>
            !matches!(operand.as_ref(), ScalarExpression::Integer { .. }),
        _ => false,
    }
}

#[cfg(feature = "internal-checker")]
fn nested_authored_integer_return<'a>(
    root: &'a ScalarExpression, root_ordinal: OccurrenceOrdinal, structure: &OccurrenceStructure,
    ty: &ScalarType, min: &BigInt, max: &BigInt,
) -> Option<BigInt> {
    type Node<'a> = (&'a ScalarExpression, OccurrenceOrdinal);
    enum Frame<'a> {
        Left { right: Node<'a>, operation: AuthoredIntegerOperation },
        Right { left: Cow<'a, BigInt>, operation: AuthoredIntegerOperation },
        Negate,
    }
    let fits = |value: &BigInt| value >= min && value <= max;
    let mut stack = Vec::new();
    let mut next = (root, root_ordinal);
    loop {
        let (expression, ordinal) = next;
        if structure.checked_types.expressions.get(&ordinal) != Some(ty) {
            return None;
        }
        let child = |edge| match structure.edges.get(&(OccurrenceParent::Expression(ordinal), edge)) {
            Some(OccurrenceNode::Expression(ordinal)) => Some(*ordinal),
            _ => None,
        };
        if let ScalarExpression::Binary { operator, left, right, .. } = expression {
            let operation = AuthoredIntegerOperation::checked(operator)?;
            stack.push(Frame::Left { right: (right, child(OccurrenceEdge::BinaryRight)?), operation });
            next = (left, child(OccurrenceEdge::BinaryLeft)?);
            continue;
        }
        if let ScalarExpression::Unary { operator: UnaryOperator::Negate, operand, .. } = expression {
            if !matches!(operand.as_ref(), ScalarExpression::Integer { .. }) {
                if min.sign() != num_bigint::Sign::Minus {
                    return None;
                }
                stack.push(Frame::Negate);
                next = (operand, child(OccurrenceEdge::UnaryOperand)?);
                continue;
            }
        }
        let mut value = concrete_integer_literal(expression)?;
        if !fits(&value) {
            return None;
        }
        loop {
            match stack.pop() {
                Some(Frame::Left { right, operation }) => {
                    stack.push(Frame::Right { left: value, operation });
                    next = right;
                    break;
                }
                Some(Frame::Right { left, operation }) => {
                    let computed = operation.apply(left, value, min)?;
                    if !fits(&computed) {
                        return None;
                    }
                    value = Cow::Owned(computed);
                }
                Some(Frame::Negate) => {
                    let computed = match value {
                        Cow::Borrowed(value) => -value,
                        Cow::Owned(value) => -value,
                    };
                    if !fits(&computed) {
                        return None;
                    }
                    value = Cow::Owned(computed);
                }
                None => return Some(value.into_owned()),
            }
        }
    }
}

#[cfg(all(test, feature = "internal-checker"))]
thread_local! {
    pub(super) static AUTHORED_RETURN_CALCULATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[derive(Debug, PartialEq, Eq)]
enum ProjectedEntry {
    Linked(usize),
    Detached(Rc<IntegerSet>),
}

#[derive(Debug, PartialEq, Eq)]
struct IntegerAffineTransform {
    reflected: bool,
    offset: BigInt,
}

impl IntegerAffineTransform {
    fn apply(&self, domain: &IntegerSet, inverse: bool) -> IntegerSet {
        domain.affine_unit(self.reflected, &self.offset, inverse)
    }

    fn inverse(&self) -> Self {
        Self { reflected: self.reflected, offset: if self.reflected { self.offset.clone() } else { -&self.offset } }
    }

    // self(inner(x))
    fn compose(&self, inner: &Self) -> Self {
        Self {
            reflected: self.reflected ^ inner.reflected,
            offset: if self.reflected { &self.offset - &inner.offset } else { &self.offset + &inner.offset },
        }
    }

    fn cancels(&self, inner: &Self) -> bool {
        if self.reflected != inner.reflected { return false; }
        if self.reflected { return self.offset == inner.offset; }
        use num_bigint::Sign::{Minus, NoSign, Plus};
        matches!((self.offset.sign(), inner.offset.sign()), (Minus, Plus) | (Plus, Minus) | (NoSign, NoSign))
            && self.offset.magnitude() == inner.offset.magnitude()
    }

    fn fits(&self, domain: &IntegerSet, minimum: &BigInt, maximum: &BigInt) -> Option<()> {
        let (first, last) = domain.extremes()?;
        let (first, last) = if self.reflected {
            (&self.offset - last, &self.offset - first)
        } else {
            (first + &self.offset, last + &self.offset)
        };
        (first >= *minimum && last <= *maximum).then_some(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct IntegerAffineAxes {
    anchor: usize,
    // Each member is ±anchor + offset; appended axes keep this sorted.
    members: Vec<(usize, Rc<IntegerAffineTransform>)>,
}

const MAX_AFFINE_MEMBERS: usize = 8;

impl IntegerAffineAxes {
    fn member(&self, axis: usize) -> Option<&Rc<IntegerAffineTransform>> {
        let i = self.members.binary_search_by_key(&axis, |(member, _)| *member).ok()?;
        Some(&self.members[i].1)
    }

    fn contains(&self, axis: usize) -> bool {
        axis == self.anchor || self.member(axis).is_some()
    }

    fn between(&self, from: usize, to: usize) -> Option<Rc<IntegerAffineTransform>> {
        if from == to { return None; }
        if from == self.anchor { return self.member(to).cloned(); }
        let inverse = self.member(from)?.inverse();
        Some(Rc::new(if to == self.anchor { inverse } else { self.member(to)?.compose(&inverse) }))
    }
}

enum AffineAppend {
    Alias(usize),
    Member { anchor: usize, transform: Rc<IntegerAffineTransform> },
}

enum EqualOverwrite {
    Unchanged,
    RemoveAlias(usize),
    Promote { index: usize, axis: usize },
}

#[cfg(feature = "internal-checker")]
enum AffineOverwrite {
    Unchanged,
    Replace(Option<Rc<IntegerAffineAxes>>),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct IntegerRow {
    cells: Vec<Rc<IntegerSet>>,
    equal_axes: Option<Rc<Vec<(usize, usize)>>>,
    affine: Option<Rc<IntegerAffineAxes>>,
    entry: Option<Rc<ProjectedEntry>>,
    projected_from: u16,
}

impl IntegerRow {
    fn new(cells: Vec<Rc<IntegerSet>>) -> Self { Self { cells, equal_axes: None, affine: None, entry: None, projected_from: 0 } }

    fn equal_axes(&self) -> &[(usize, usize)] {
        self.equal_axes.as_deref().map_or(&[], Vec::as_slice)
    }

    fn canonical_axis(&self, axis: usize) -> usize {
        let equality = self.equal_axes();
        equality.binary_search_by_key(&axis, |(alias, _)| *alias).ok()
            .map_or(axis, |index| equality[index].1)
    }

    fn append_alias(&mut self, from: usize, domain: Rc<IntegerSet>) {
        let canonical = self.canonical_axis(from);
        let next = self.cells.len();
        Rc::make_mut(self.equal_axes.get_or_insert_with(|| Rc::new(Vec::new()))).push((next, canonical));
        self.cells.push(domain);
    }

    fn prepare_affine(
        &self, from: usize, domain: &IntegerSet, transform: &Rc<IntegerAffineTransform>, work: &mut usize,
    ) -> Option<AffineAppend> {
        let from = self.canonical_axis(from);
        let (anchor, count, inner) = match self.affine.as_deref() {
            Some(class) if from != class.anchor =>
                (class.anchor, class.members.len(), Some(class.member(from)?)),
            Some(class) => (from, class.members.len(), None),
            None => (from, 0, None),
        };
        let identity = inner.map_or_else(
            || !transform.reflected && transform.offset.bits() == 0,
            |inner| transform.cancels(inner),
        );
        if identity {
            charge_call_work(work, self.equal_axes().len().checked_add(1)?)?;
            return Some(AffineAppend::Alias(anchor));
        }
        if count == MAX_AFFINE_MEMBERS { return None; }
        // Charge mapped intervals, copy-on-write members, new member/domain
        // storage, and any composed transform before allocating the plan.
        let stored = if inner.is_some() { 4 } else { 3 };
        charge_call_work(work, domain.interval_count().checked_add(count)?.checked_add(stored)?)?;
        let transform = inner.map_or_else(|| Rc::clone(transform), |inner| Rc::new(transform.compose(inner)));
        Some(AffineAppend::Member { anchor, transform })
    }

    fn append_affine(&mut self, plan: AffineAppend) {
        match plan {
            AffineAppend::Alias(anchor) => self.append_alias(anchor, Rc::clone(&self.cells[anchor])),
            AffineAppend::Member { anchor, transform } => {
                let class = self.affine.get_or_insert_with(|| Rc::new(IntegerAffineAxes { anchor, members: Vec::new() }));
                debug_assert!(class.anchor == anchor && class.members.len() < MAX_AFFINE_MEMBERS);
                let output = self.cells.len();
                let domain = Rc::new(transform.apply(&self.cells[anchor], false));
                Rc::make_mut(class).members.push((output, transform));
                self.cells.push(domain);
            }
        }
    }

    #[cfg(test)]
    fn test_append_affine(&mut self, from: usize, transform: Rc<IntegerAffineTransform>) -> Option<()> {
        let mut work = 0;
        let plan = self.prepare_affine(from, &self[from], &transform, &mut work)?;
        self.append_affine(plan);
        Some(())
    }

    fn affine_touches(&self, axis: usize) -> bool {
        self.affine.as_ref().is_some_and(|class| class.contains(self.canonical_axis(axis)))
    }

    fn charge_affine_replacement(&self, axis: usize, intervals: usize, work: &mut usize) -> Option<()> {
        let axes = self.affine.as_ref().filter(|class| class.contains(self.canonical_axis(axis)))
            .map_or(0, |class| class.members.len());
        if axes != 0 {
            *work = work.checked_add(intervals.checked_add(self.equal_axes().len())?.checked_mul(axes)?)?;
        }
        Some(())
    }

    fn replace_domain(&mut self, index: usize, set: Rc<IntegerSet>) {
        let canonical = self.canonical_axis(index);
        if let Some(class) = self.affine.as_ref().filter(|class| class.contains(canonical)).cloned() {
            let anchor_domain = if let Some(transform) = class.member(canonical) {
                let anchor = Rc::new(transform.apply(&set, true));
                self.replace_equal_group(class.anchor, Rc::clone(&anchor));
                anchor
            } else {
                Rc::clone(&set)
            };
            for (member, transform) in &class.members {
                if *member != canonical {
                    self.replace_equal_group(*member, Rc::new(transform.apply(&anchor_domain, false)));
                }
            }
        }
        self.replace_equal_group(canonical, set);
    }

    fn linked_axes(&self, first: usize, second: usize) -> bool {
        let first = self.canonical_axis(first);
        let second = self.canonical_axis(second);
        first == second || self.affine.as_ref().is_some_and(|class| class.contains(first) && class.contains(second))
    }

    fn charge_affine_pair(&self, [first, second]: [usize; 2], [left, right]: [usize; 2], work: &mut usize) -> Option<()> {
        if self.affine.is_some() {
            self.charge_affine_replacement(first, left, work)?;
            if !self.linked_axes(first, second) {
                self.charge_affine_replacement(second, right, work)?;
            }
        }
        Some(())
    }

    // The pair receipt is anchored to this caller row. Reconstruct the first
    // restriction, then validate rather than overwrite the second if linked.
    fn replace_pair(&mut self, [first, second]: [usize; 2], [left, right]: [Rc<IntegerSet>; 2]) -> Option<()> {
        self.replace_domain(first, left);
        if self.linked_axes(first, second) {
            (self[second] == right).then_some(())
        } else {
            self.replace_domain(second, right);
            Some(())
        }
    }

    fn replace_equal_group(&mut self, canonical: usize, set: Rc<IntegerSet>) {
        if let Some(equality) = &self.equal_axes {
            for &(alias, root) in equality.iter() {
                if root == canonical {
                    self.cells[alias] = Rc::clone(&set);
                }
            }
        }
        self.cells[canonical] = set;
    }

    fn entry_domain(&self, formal: usize) -> &Rc<IntegerSet> {
        match self.entry.as_deref() {
            None => &self.cells[formal],
            Some(ProjectedEntry::Linked(axis)) => &self.cells[*axis],
            Some(ProjectedEntry::Detached(domain)) => domain,
        }
    }

    fn entry_alias(&self, formal: usize, axis: usize) -> bool {
        let canonical = self.canonical_axis(axis);
        match self.entry.as_deref() {
            None => canonical == formal,
            Some(ProjectedEntry::Linked(root)) => canonical == *root,
            Some(ProjectedEntry::Detached(_)) => false,
        }
    }

    fn entry_affine(&self, formal: usize, axis: usize) -> Option<Rc<IntegerAffineTransform>> {
        let class = self.affine.as_deref()?;
        let from = match self.entry.as_deref() {
            None => formal,
            Some(ProjectedEntry::Linked(root)) => *root,
            Some(ProjectedEntry::Detached(_)) => return None,
        };
        class.between(from, self.canonical_axis(axis))
    }

    fn entry_key(&self) -> Option<Result<usize, *const IntegerSet>> {
        self.entry.as_deref().map(|entry| match entry {
            ProjectedEntry::Linked(axis) => Ok(*axis),
            ProjectedEntry::Detached(domain) => Err(Rc::as_ptr(domain)),
        })
    }

    fn equal_overwrite(&self, axis: usize) -> EqualOverwrite {
        let Some(equality) = &self.equal_axes else { return EqualOverwrite::Unchanged; };
        if let Ok(index) = equality.binary_search_by_key(&axis, |(alias, _)| *alias) {
            EqualOverwrite::RemoveAlias(index)
        } else if let Some(index) = equality.iter().position(|&(_, root)| root == axis) {
            EqualOverwrite::Promote { index, axis: equality[index].0 }
        } else {
            EqualOverwrite::Unchanged
        }
    }

    fn tracks_overwrite(&self, axis: usize, preserve_entry: bool) -> bool {
        (preserve_entry && self.entry.is_none())
            || matches!(self.entry.as_deref(), Some(ProjectedEntry::Linked(root)) if *root == axis)
    }

    fn overwrite_prepared(&mut self, axis: usize, domain: Rc<IntegerSet>, preserve_entry: bool, equal: EqualOverwrite) {
        let replacement_root = match equal {
            EqualOverwrite::Unchanged => None,
            EqualOverwrite::RemoveAlias(index) => {
                let equality = Rc::make_mut(self.equal_axes.as_mut().expect("prepared alias"));
                equality.remove(index);
                if equality.is_empty() { self.equal_axes = None; }
                None
            }
            EqualOverwrite::Promote { index, axis: promoted } => {
                let equality = Rc::make_mut(self.equal_axes.as_mut().expect("prepared alias promotion"));
                equality.remove(index);
                for (_, root) in &mut equality[index..] {
                    if *root == axis { *root = promoted; }
                }
                if equality.is_empty() { self.equal_axes = None; }
                Some(promoted)
            }
        };
        if self.tracks_overwrite(axis, preserve_entry) {
            self.entry = Some(Rc::new(replacement_root.map_or_else(
                || ProjectedEntry::Detached(Rc::clone(&self.cells[axis])), ProjectedEntry::Linked)));
        }
        self.cells[axis] = domain;
    }

    fn overwrite(&mut self, axis: usize, domain: Rc<IntegerSet>, preserve_entry: bool) {
        self.overwrite_prepared(axis, domain, preserve_entry, self.equal_overwrite(axis));
    }

    #[cfg(feature = "internal-checker")]
    fn prepare_affine_overwrite(
        &self, axis: usize, preserve_entry: bool, work: &mut usize,
    ) -> Option<(EqualOverwrite, AffineOverwrite)> {
        let aliases = self.equal_axes().len();
        charge_call_work(work, aliases.checked_add(1)?)?;
        let equal = self.equal_overwrite(axis);
        if !matches!(&equal, EqualOverwrite::Unchanged) {
            // Copy-on-write, removal and possible retargeting of the group.
            charge_call_work(work, aliases.checked_mul(3)?)?;
        }
        let Some(class) = self.affine.as_deref().filter(|class| class.contains(axis)) else {
            // A destination alias is not itself a class endpoint. Detaching it
            // leaves the pre-write class and its other aliases unchanged.
            return Some((equal, AffineOverwrite::Unchanged));
        };
        let promoted = match &equal {
            EqualOverwrite::Promote { axis, .. } => Some(*axis),
            _ => None,
        };
        if promoted.is_none() && self.tracks_overwrite(axis, preserve_entry) {
            // No distinct lexical snapshot denotes the old projected formal.
            // A detached marginal cannot prove correlation with surviving values.
            return None;
        }
        let survivors = class.members.len().checked_add(usize::from(promoted.is_some()))?;
        if survivors <= 1 {
            return Some((equal, AffineOverwrite::Replace(None)));
        }
        // At most MAX_AFFINE_MEMBERS+1 endpoints; pay for the list,
        // ordering, one inverse, and at most one composition per endpoint.
        let order_levels = usize::BITS - survivors.leading_zeros();
        charge_call_work(work, survivors.checked_mul(4 + order_levels as usize)?.checked_add(2)?)?;
        let mut ordered: Vec<_> = std::iter::once(class.anchor)
            .chain(class.members.iter().map(|(member, _)| *member))
            .filter_map(|old_axis| {
                if old_axis == axis {
                    promoted.map(|new_axis| (new_axis, old_axis))
                } else {
                    Some((old_axis, old_axis))
                }
            }).collect();
        ordered.sort_unstable_by_key(|&(new_axis, _)| new_axis);
        let ((anchor, old_anchor), members) = ordered.split_first()?;
        let inverse = if *old_anchor == class.anchor { None } else { Some(Rc::new(class.member(*old_anchor)?.inverse())) };
        let members = members.iter().map(|&(new_axis, old_axis)| {
            let transform = match &inverse {
                None => Rc::clone(class.member(old_axis)?),
                Some(inverse) if old_axis == class.anchor => Rc::clone(inverse),
                Some(inverse) => Rc::new(class.member(old_axis)?.compose(inverse)),
            };
            Some((new_axis, transform))
        }).collect::<Option<Vec<_>>>()?;
        Some((equal, AffineOverwrite::Replace(Some(Rc::new(IntegerAffineAxes { anchor: *anchor, members })))))
    }

    fn truncate(&mut self, depth: usize) {
        if let Some(ProjectedEntry::Linked(axis)) = self.entry.as_deref() {
            if *axis >= depth {
                self.entry = Some(Rc::new(ProjectedEntry::Detached(Rc::clone(&self.cells[*axis]))));
            }
        }
        if let Some(class) = &mut self.affine {
            let keep = class.members.partition_point(|(member, _)| *member < depth);
            if keep == 0 {
                self.affine = None;
            } else if keep < class.members.len() {
                Rc::make_mut(class).members.truncate(keep);
            }
        }
        self.cells.truncate(depth);
        if let Some(equality) = &mut self.equal_axes {
            let keep = equality.partition_point(|(alias, _)| *alias < depth);
            if keep == 0 {
                self.equal_axes = None;
            } else if keep != equality.len() {
                Rc::make_mut(equality).truncate(keep);
            }
        }
    }
}

impl std::ops::Deref for IntegerRow {
    type Target = Vec<Rc<IntegerSet>>;
    fn deref(&self) -> &Self::Target { &self.cells }
}

impl std::ops::DerefMut for IntegerRow {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.cells }
}

impl FromIterator<Rc<IntegerSet>> for IntegerRow {
    fn from_iter<T: IntoIterator<Item = Rc<IntegerSet>>>(iter: T) -> Self {
        Self::new(iter.into_iter().collect())
    }
}

#[derive(Default)]
struct IntegerRoots<'a> {
    axes: Vec<(&'a ReferenceBindingId, &'a ScalarType)>,
    by_binding: HashMap<&'a ReferenceBindingId, usize>,
    parameters: usize,
    projected_entry_axis: Option<usize>,
    unit_pair_work: Option<usize>,
    affine_work: Option<usize>,
    #[cfg(feature = "internal-checker")]
    allow_ordinary_integer_locals: bool,
    #[cfg(feature = "internal-checker")]
    allow_ordinary_affine_overwrites: bool,
    #[cfg(feature = "internal-checker")]
    ordinary_local_work: Option<usize>,
    #[cfg(feature = "internal-checker")]
    ordinary_comparison_work: Option<usize>,
    reached_obligation: bool,
    proof_reflexive_branches: bool,
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
    // Only visited from actual copy/relational transfer sites, never from the
    // generic drive loop. Affine rows are already metered by that lane.
    #[cfg(feature = "internal-checker")]
    fn charge_local_alias_work(&self, work: &mut Option<usize>, passes: usize) -> Option<()> {
        if let Some(work) = work {
            for row in &self.0 {
                charge_call_work(work, 1)?;
                if row.affine.is_none() && row.equal_axes.is_some() {
                    charge_call_work(work, row.equal_axes().len().checked_mul(passes)?)?;
                }
            }
        }
        Some(())
    }

    #[cfg(any(test, feature = "internal-checker"))]
    fn from_cells(rows: Vec<Vec<Rc<IntegerSet>>>) -> Self {
        Self(rows.into_iter().map(IntegerRow::new).collect())
    }

    fn axis_has_alias(&self, axis: usize) -> bool {
        self.0.iter().any(|row| {
            let canonical = row.canonical_axis(axis);
            row.equal_axes().iter().any(|(_, root)| *root == canonical)
        })
    }

    fn common_equal_axes(&self) -> Vec<(usize, usize)> {
        let Some((first, rest)) = self.0.split_first() else { return Vec::new(); };
        first.equal_axes().iter().copied().filter(|&(axis, canonical)|
            rest.iter().all(|row| row.canonical_axis(axis) == row.canonical_axis(canonical))
        ).collect()
    }

    #[cfg(feature = "internal-checker")]
    fn common_equal_axes_bounded(&self, work: &mut usize) -> Option<Vec<(usize, usize)>> {
        let Some((first, rest)) = self.0.split_first() else { return Some(Vec::new()); };
        let Some(aliases) = &first.equal_axes else { return Some(Vec::new()); };
        charge_call_work(work, aliases.len().checked_add(1)?)?;
        let mut common = aliases.as_ref().clone();
        for row in rest {
            if common.is_empty() { break; }
            charge_call_work(work, 1)?;
            let Some(other) = &row.equal_axes else { common.clear(); break; };
            if Rc::ptr_eq(aliases, other) { continue; }
            let lookups = usize::BITS - other.len().leading_zeros();
            charge_call_work(work, common.len().checked_mul(1 + 2 * lookups as usize)?)?;
            common.retain(|&(axis, canonical)| row.canonical_axis(axis) == row.canonical_axis(canonical));
        }
        Some(common)
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn row_equal_axes(&self, row: usize) -> Option<&[(usize, usize)]> {
        Some(self.0.get(row)?.equal_axes())
    }

    pub(super) fn closed() -> Self {
        Self(vec![IntegerRow::default()])
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn entry_point(earlier: Option<&BigInt>, value: &BigInt) -> Self {
        Self(vec![earlier.into_iter().chain([value])
            .map(|value| Rc::new(IntegerSet::singleton(value))).collect()])
    }

    fn singleton_at(&self, axis: usize) -> Option<&BigInt> {
        self.singleton_axis(axis).map(|(value, _)| value)
    }

    fn singleton_axis(&self, axis: usize) -> Option<(&BigInt, &Rc<IntegerSet>)> {
        self.singleton_axis_with(axis, || Some(()))
    }

    fn singleton_axis_with(
        &self, axis: usize, mut inspect: impl FnMut() -> Option<()>,
    ) -> Option<(&BigInt, &Rc<IntegerSet>)> {
        let (first, rest) = self.0.split_first()?;
        inspect()?;
        let domain = &first[axis];
        let value = domain.singleton_value()?;
        rest.iter().all(|row| inspect().is_some() && row[axis].singleton_value() == Some(value))
            .then_some((value, domain))
    }

    #[cfg(feature = "internal-checker")]
    fn singleton_pair_at(&self, left: usize, right: usize) -> Option<(&BigInt, &BigInt)> {
        let (first, rest) = self.0.split_first()?;
        let left_value = first[left].singleton_value()?;
        let right_value = first[right].singleton_value()?;
        rest.iter().all(|row| {
            row[left].singleton_value() == Some(left_value) && row[right].singleton_value() == Some(right_value)
        }).then_some((left_value, right_value))
    }

    fn project_axis(&self, axis: usize) -> Option<Rc<IntegerSet>> {
        self.project_domain(|row| &row[axis])
    }

    fn project_entry(&self, formal: usize) -> Option<Rc<IntegerSet>> {
        self.project_domain(|row| row.entry_domain(formal))
    }

    fn project_domain(&self, domain: impl Fn(&IntegerRow) -> &Rc<IntegerSet>) -> Option<Rc<IntegerSet>> {
        let (first, rest) = self.0.split_first()?;
        let first = domain(first);
        Some(if rest.iter().all(|row| Rc::ptr_eq(first, domain(row))) {
            Rc::clone(first)
        } else {
            Rc::new(IntegerSet::union(self.0.iter().map(|row| domain(row).as_ref())))
        })
    }

    fn restrict_axis(&mut self, axis: usize, survivors: &IntegerSet) {
        self.0.retain_mut(|row| match row[axis].intersection(survivors) {
            IntegerIntersection::Unchanged => true,
            IntegerIntersection::Empty => false,
            IntegerIntersection::Changed(retained) => {
                row.replace_domain(axis, Rc::new(retained));
                true
            }
        });
    }

    fn charge_affine_disjunction(
        &self, axis: usize, cuts: &IntegerCutUnion<'_>, require: bool, work: &mut usize,
    ) -> Option<()> {
        let uniform = cuts.uniform();
        let extra = cuts.max_added_intervals().saturating_sub(1);
        let copies = require && uniform != Some(true);
        if extra == 0 && !copies {
            return Some(());
        }
        for row in &self.0 {
            if row.affine.is_none() {
                continue;
            }
            if copies {
                *work = work.checked_add(row.len())?;
            }
            if extra != 0 && row.affine_touches(axis) {
                *work = work.checked_add(extra.checked_mul(4)?.checked_mul(row.affine.as_ref()?.members.len())?)?;
            }
            if *work > MAX_PARTITION_TRANSFER_WORK {
                return None;
            }
        }
        Some(())
    }

    fn disjunction_failure_witness(&self, axis: usize, cuts: &IntegerCutUnion<'_>) -> Self {
        if let Some(passes) = cuts.uniform() {
            return if passes { Self::default() } else { Self(self.0.clone()) };
        }
        let mut witness = Self::default();
        for row in &self.0 {
            match row[axis].filter_by(cuts, false) {
                IntegerIntersection::Empty => {}
                IntegerIntersection::Unchanged => witness.0.push(row.clone()),
                IntegerIntersection::Changed(intersection) => {
                    let mut row = row.clone();
                    row.replace_domain(axis, Rc::new(intersection));
                    witness.0.push(row);
                }
            }
        }
        witness
    }

    fn narrow_disjunction_or_witness(&mut self, axis: usize, cuts: &IntegerCutUnion<'_>) -> Self {
        if let Some(passes) = cuts.uniform() {
            return if passes { Self::default() } else { std::mem::take(self) };
        }
        let mut rejected_prefix = 0;
        let mut surviving = false;
        self.0.retain_mut(|row| match row[axis].filter_by(cuts, true) {
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
                row.replace_domain(axis, Rc::new(intersection));
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
        let (passing, failing) = self.split_equal_axis(index, &predicate.operator, &predicate.bound)?;
        Some((passing, failing, Some(index)))
    }

    #[cfg(feature = "internal-checker")]
    fn split_axis(self, index: usize, operator: &BinaryOperator, bound: &BigInt) -> Option<(Self, Self)> {
        self.split_equal_axis(index, operator, bound)
    }

    fn split_equal_axis(self, index: usize, operator: &BinaryOperator, bound: &BigInt) -> Option<(Self, Self)> {
        let mut passing = Self::default();
        let mut failing = Self::default();
        for mut row in self.0 {
            let (yes, no) = row[index].split(operator, bound)?;
            if yes.is_empty() {
                failing.0.push(row);
            } else if no.is_empty() {
                passing.0.push(row);
            } else {
                let mut alternate = row.clone();
                row.replace_domain(index, Rc::new(yes));
                alternate.replace_domain(index, Rc::new(no));
                passing.0.push(row);
                failing.0.push(alternate);
            }
        }
        Some((passing, failing))
    }

    #[cfg(feature = "internal-checker")]
    fn split_partition(mut self, index: usize, accepted: &IntegerCutUnion<'_>) -> (Self, Self) {
        use super::symbolic_integer::IntegerSetPartition;
        match accepted.uniform() {
            Some(true) => return (self, Self::default()),
            Some(false) => return (Self::default(), self),
            None => {}
        }
        let mut retained_side = None;
        let mut other = Self::default();
        self.0.retain_mut(|row| {
            let passing = match row[index].partition_by(accepted) {
                IntegerSetPartition::True => true,
                IntegerSetPartition::False => false,
                IntegerSetPartition::Mixed(yes, no) => {
                    let (retained, moved) = if *retained_side.get_or_insert(true) { (yes, no) } else { (no, yes) };
                    let mut alternate = row.clone();
                    row.replace_domain(index, Rc::new(retained));
                    alternate.replace_domain(index, Rc::new(moved));
                    other.0.push(alternate);
                    return true;
                }
            };
            if passing == *retained_side.get_or_insert(passing) {
                true
            } else {
                other.0.push(std::mem::take(row));
                false
            }
        });
        if retained_side == Some(false) { (other, self) } else { (self, other) }
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
                    row.replace_domain(index, Rc::new(rejected));
                }
                failing.0.push(row);
            }
        }
        Some(failing)
    }

    #[cfg(feature = "internal-checker")]
    fn join(self, other: Self, axis: Option<usize>) -> Self {
        self.join_equal(other, axis)
    }

    fn join_equal(mut self, mut other: Self, axis: Option<usize>) -> Self {
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
            let canonical = left.canonical_axis(axis);
            if left.affine.is_none() && right.affine.is_none()
                && left.projected_from == right.projected_from && left.entry_key() == right.entry_key() && left.equal_axes() == right.equal_axes() && left.iter().zip(right.iter()).enumerate().all(|(index, (a, b))| {
                left.canonical_axis(index) == canonical || Rc::ptr_eq(a, b)
            }) {
                let right = other.0.pop().unwrap();
                merge_axis(left, right, canonical);
            } else {
                self.0.append(&mut other.0);
            }
            return self;
        }

        Self::compact_axis(self.0.into_iter().chain(other.0), axis)
    }

    fn overwrite(mut self, axis: usize, value: Rc<IntegerSet>, preserve_entry: bool) -> Self {
        for row in &mut self.0 {
            row.overwrite(axis, Rc::clone(&value), preserve_entry);
        }
        if self.0.len() < 2 {
            return self;
        }
        Self::compact_axis(self.0, axis)
    }

    #[cfg(feature = "internal-checker")]
    fn overwrite_affine(&mut self, axis: usize, value: &Rc<IntegerSet>, preserve_entry: bool, work: &mut usize) -> Option<()> {
        for row in &mut self.0 {
            let (equal, affine) = row.prepare_affine_overwrite(axis, preserve_entry, work)?;
            row.overwrite_prepared(axis, Rc::clone(value), preserve_entry, equal);
            if let AffineOverwrite::Replace(new) = affine {
                row.affine = new;
            }
        }
        // Keep source row order even if rewriting a row makes a relation
        // disappear. Captured/anchored failure structure must not coalesce.
        Some(())
    }

    fn append_local(&mut self, values: IntegerSet) {
        self.append_shared_local(&Rc::new(values));
    }

    fn append_shared_local(&mut self, values: &Rc<IntegerSet>) {
        for row in &mut self.0 {
            row.push(Rc::clone(values));
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
            if row.affine.is_some() {
                joined.0.push(row);
                continue;
            }
            let canonical = row.canonical_axis(axis);
            let values: Vec<_> = if row.equal_axes.is_none() {
                row[..axis].iter().chain(&row[axis + 1..]).map(Rc::as_ptr).collect()
            } else {
                row.iter().enumerate().filter_map(|(index, domain)|
                    (row.canonical_axis(index) != canonical).then(|| Rc::as_ptr(domain))).collect()
            };
            match representatives.entry((row.projected_from, row.entry_key(), row.equal_axes.clone(), values)) {
                std::collections::hash_map::Entry::Occupied(entry) => {
                    let target = &mut joined.0[*entry.get()];
                    merge_axis(target, row, canonical);
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
                && row.equal_axes().iter().all(|&(alias, root)| Into::<BigInt>::into(coordinates[alias]) == Into::<BigInt>::into(coordinates[root]))
                && row.affine.as_ref().is_none_or(|class| class.members.iter().all(|(member, transform)| {
                    let input: BigInt = coordinates[class.anchor].into();
                    let output: BigInt = coordinates[*member].into();
                    output == if transform.reflected { &transform.offset - input } else { input + &transform.offset }
                }))
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
    let replacement = row.cells.into_iter().nth(axis).expect("canonical integer axis");
    let previous = std::mem::replace(&mut representative[axis], replacement);
    let target = Rc::make_mut(&mut representative[axis]);
    *target = Rc::unwrap_or_clone(previous).join(std::mem::take(target));
    if representative.equal_axes.is_some() {
        let domain = Rc::clone(&representative[axis]);
        representative.replace_domain(axis, domain);
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct IntegerCheckFailure<'a> {
    pub(super) directive: OccurrenceOrdinal,
    pub(super) state: IntegerInputs,
    pub(super) axes: Vec<(&'a ReferenceBindingId, &'a ScalarType)>,
    pub(super) equal_axes: Vec<(usize, usize)>,
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
    span: super::ByteSpan,
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
        &self, paths: &[IntegerReturningPath<'_>], domain: &IntegerSet, output: Option<&IntegerShowConstraints<'_>>,
    ) -> Option<bool> {
        let equal = IntegerRelation::from_operator(&BinaryOperator::Equal)?;
        let reversed = self.relation.reversed();
        let reflexive = self.relation.accepts(std::cmp::Ordering::Equal);
        for path in paths {
            let witness = path.axes.iter().position(|known| *known == self.binding)?;
            let (returned, comparison) = match path.value {
                IntegerReturn::Const(value) => {
                    if output.is_some_and(|bounds| !bounds.holds_value(value)) {
                        return Some(false);
                    }
                    for row in &path.state.0 {
                        let candidate = &row[witness];
                        let matches = if self.relation == equal {
                            candidate.singleton_value() == Some(value)
                        } else {
                            candidate.every_at_extremes(reversed, value)
                        };
                        if !matches || !candidate.is_subset_of(domain) {
                            return Some(false);
                        }
                    }
                    continue;
                }
                IntegerReturn::Alias(binding) if binding != self.binding => {
                    let (other, comparison) = self.other?;
                    if binding != other {
                        return None;
                    }
                    (path.axes.iter().position(|known| *known == other)?, Some(comparison))
                }
                IntegerReturn::Alias(_) if reflexive => (witness, None),
                IntegerReturn::Alias(_) => return Some(false),
            };
            let holds = match output {
                Some(bounds) => Self::holds_existential_alias_rows(path, witness, returned, comparison, domain,
                    |values| bounds.holds_set(values)),
                None => Self::holds_existential_alias_rows(path, witness, returned, comparison, domain, |_| true),
            };
            if !holds? {
                return Some(false);
            }
        }
        Some(true)
    }

    fn holds_existential_alias_rows(
        path: &IntegerReturningPath<'_>, witness: usize, returned: usize,
        comparison: Option<CartesianRelation>, domain: &IntegerSet, bounds_hold: impl Fn(&IntegerSet) -> bool,
    ) -> Option<bool> {
        for row in &path.state.0 {
            let candidate = &row[witness];
            let output = &row[returned];
            if let Some(comparison) = comparison {
                if !comparison.holds(output, candidate)? {
                    return Some(false);
                }
            }
            if !bounds_hold(output) || !candidate.is_subset_of(domain) {
                return Some(false);
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
    fn singleton_return(&self) -> Option<&BigInt> {
        match self.value {
            IntegerReturn::Const(value) => Some(value),
            IntegerReturn::Alias(binding) => self.state.singleton_at(self.axes.iter().position(|id| *id == binding)?),
        }
    }

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
    pub(super) fn output_disjoint_from(&self, domain: &IntegerSet) -> Option<bool> {
        match self.value {
            IntegerReturn::Const(value) => Some(!domain.covers_closed(value, value)),
            IntegerReturn::Alias(binding) => {
                let axis = self.axes.iter().position(|known| *known == binding)?;
                Some(self.state.0.iter().all(|row| row[axis].is_disjoint(domain)))
            }
        }
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn output_satisfies_constraints_avoiding(
        &self, output: &IntegerShowConstraints<'_>, forbidden: &IntegerSet,
    ) -> Option<bool> {
        match self.value {
            IntegerReturn::Const(value) => Some(output.holds_value(value) && !forbidden.covers_closed(value, value)),
            IntegerReturn::Alias(binding) => {
                let axis = self.axes.iter().position(|known| *known == binding)?;
                Some(self.state.0.iter().all(|row| output.holds_set_avoiding(&row[axis], forbidden)))
            }
        }
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn output_satisfies_constraints(&self, output: &IntegerShowConstraints<'_>) -> Option<bool> {
        match self.value {
            IntegerReturn::Const(value) => Some(output.holds_value(value)),
            IntegerReturn::Alias(binding) => {
                let axis = self.axes.iter().position(|known| *known == binding)?;
                Some(self.state.0.iter().all(|row| output.holds_set(&row[axis])))
            }
        }
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn output_satisfies_existential_literal(
        &self, relation: IntegerRelation, witness: &BigInt, output: &IntegerShowConstraints<'_>,
    ) -> Option<bool> {
        match self.value {
            IntegerReturn::Const(value) => Some(relation.accepts(value.cmp(witness)) && output.holds_value(value)),
            IntegerReturn::Alias(binding) => {
                let axis = self.axes.iter().position(|known| *known == binding)?;
                Some(self.state.0.iter().all(|row|
                    row[axis].every_at_extremes(relation, witness) && output.holds_set(&row[axis])))
            }
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
                self.state.split_axis(index, &predicate.operator, &predicate.bound)?
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
    PendingCall(ExactCallEffect<'a>),
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
    run_integer_callable::<false, false, _>(calls, integer_tail_type(calls), None, None, &mut [], None, None, None, false, ())
        .map(|result| result.map(|coverage| matches!(coverage, ProofCoverage::Applicable(true))))
}

pub(super) fn collect_integer_requirement_failures(
    calls: &FunctionCallSites,
) -> Option<(bool, Vec<IntegerCheckFailure<'_>>)> {
    let mut failures = Vec::new();
    let mut sink = |failure| failures.push(failure);
    let (active, _) = collect_integer_requirement_failures_with_calls(calls, IntegerRequirementEntry::Arbitrary, None, false, false, &mut sink)?;
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
    allow_contextual_local_transfers: bool,
    sink: &'sink mut dyn FnMut(IntegerCheckFailure<'a>),
}

pub(super) fn collect_integer_requirement_failures_with_calls<'a>(
    calls: &'a FunctionCallSites,
    entry: IntegerRequirementEntry<'_>,
    handle_call: Option<&mut ActualCallHandler<'a, '_>>,
    allow_projected_calls: bool,
    allow_contextual_local_transfers: bool,
    sink: &mut dyn FnMut(IntegerCheckFailure<'a>),
) -> Option<(bool, IntegerRequirementContinuation)> {
    collect_integer_requirement_failures_in_scope(calls, entry, handle_call, allow_projected_calls, allow_contextual_local_transfers, sink, ())
}

#[cfg(feature = "internal-checker")]
pub(super) fn collect_ordered_root_requirement_failures<'a>(
    calls: &'a FunctionCallSites, body: &'a super::ScalarBlock, allow_ordinary_integer_locals: bool,
) -> Option<(bool, Vec<IntegerCheckFailure<'a>>)> {
    let mut failures = Vec::new();
    let mut sink = |failure| failures.push(failure);
    let (active, _) = if allow_ordinary_integer_locals && (calls.structure.has_ordinary_integer_operations()
        || ordinary_tail_type(calls, true) == Some(&ScalarType::Bool)) {
        collect_integer_requirement_failures_in_scope(
            calls, IntegerRequirementEntry::Arbitrary, None, false, false, &mut sink, OrderedRootConditions::<true, true>(body),
        )
    } else {
        collect_integer_requirement_failures_in_scope(
            calls, IntegerRequirementEntry::Arbitrary, None, false, false, &mut sink, OrderedRootConditions::<true>(body),
        )
    }?;
    Some((active, failures))
}

#[cfg(feature = "internal-checker")]
pub(super) fn collect_ordered_root_entry_equality<'a>(
    calls: &'a FunctionCallSites, body: &'a super::ScalarBlock, old: usize, predicate: Option<&IntegerPredicate>,
) -> Option<(bool, Vec<IntegerCheckFailure<'a>>, bool, Option<SymbolicLiteralFailure<'a>>)> {
    let parameters = &calls.boundary.parameters;
    let bindings = calls.boundary.checker_parameter_bindings.as_deref()?;
    if &parameters.get(old)?.ty != integer_tail_type(calls)? || parameters.len() > 2 {
        return None;
    }
    let guard = if let Some(predicate) = predicate {
        let axis = bindings.iter().position(|binding| binding == &predicate.binding)?;
        if parameters.get(axis)?.ty != predicate.ty {
            return None;
        }
        Some((axis, IntegerRelation::from_operator(&predicate.operator)?, &predicate.bound))
    } else {
        None
    };
    let observed = std::cell::RefCell::new((false, None));
    let scope = OrderedRootEntryEquality { conditions: OrderedRootConditions(body), old, guard, observed: &observed };
    let mut failures = Vec::new();
    let (active, _) = collect_integer_requirement_failures_in_scope(
        calls, IntegerRequirementEntry::Arbitrary, None, false, false, &mut |failure| failures.push(failure), scope,
    )?;
    let (returned, failure) = observed.into_inner();
    Some((active, failures, returned, failure))
}

#[cfg(feature = "internal-checker")]
pub(super) fn collect_ordered_root_literal_proof<'a, const TOTAL: bool>(
    calls: &'a FunctionCallSites, body: &'a super::ScalarBlock, relation: IntegerRelation, bound: &BigInt,
    predicate: Option<&IntegerPredicate>, entry_filter: Option<&[CheckerPredicate]>,
) -> Option<(bool, Vec<IntegerCheckFailure<'a>>, bool, Option<SymbolicLiteralFailure<'a>>)> {
    let bounds;
    let guard = if let Some(predicate) = predicate {
        let bindings = calls.boundary.checker_parameter_bindings.as_deref()?;
        let axis = bindings.iter().position(|binding| binding == &predicate.binding)?;
        let parameter = calls.boundary.parameters.get(axis)?;
        if parameter.ty != predicate.ty {
            return None;
        }
        let relation_when = IntegerRelation::from_operator(&predicate.operator)?;
        bounds = super::typecheck::integer_bounds(&parameter.ty)?;
        let (min, max) = &bounds;
        if &predicate.bound < min || &predicate.bound > max {
            return None;
        }
        let nonfailure = if integer_tail_type(calls)? == &parameter.ty {
            if bound < min || bound > max {
                return None;
            }
            Some(IntegerCutUnion::new((relation_when.complement(), &predicate.bound), (relation, bound), (min, max)))
        } else {
            None
        };
        Some(LiteralGuardAtReturn { axis, relation: relation_when, bound: &predicate.bound, nonfailure })
    } else {
        None
    };
    let observed = std::cell::RefCell::new((false, None));
    let mut failures = Vec::new();
    let mut sink = |failure| failures.push(failure);
    let scope = OrderedRootLiteralProof::<TOTAL> {
        conditions: OrderedRootConditions(body), relation, bound, guard: guard.as_ref(), observed: &observed,
    };
    let active = if TOTAL {
        let mut collected = IntegerFailureCollection {
            continuation: IntegerRequirementContinuation::Empty, projected: None, allow_projected_calls: false, allow_contextual_local_transfers: false, sink: &mut sink,
        };
        match run_integer_callable::<false, true, _>(
            calls, integer_tail_type(calls), entry_filter, None, &mut [], None, None, Some(&mut collected), false, scope,
        )?.ok()? {
            ProofCoverage::Applicable(active) => active,
            ProofCoverage::NoApplicableInputs => false,
        }
    } else {
        collect_integer_requirement_failures_in_scope(
            calls, IntegerRequirementEntry::Arbitrary, None, false, false, &mut sink, scope,
        )?.0
    };
    let (returned, failure) = observed.into_inner();
    Some((active, failures, returned, failure))
}

#[cfg(feature = "internal-checker")]
pub(super) fn collect_ordered_root_literal_proofs<'a, const STEP_FIRST: bool>(
    calls: &'a FunctionCallSites, body: &'a super::ScalarBlock,
    checks: &mut [SymbolicLiteralCheck<'_, 'a>],
) -> Option<(bool, Vec<IntegerCheckFailure<'a>>, bool)> {
    let observed = std::cell::RefCell::new(BatchedLiteralEvidence { returned: false, pending: checks.len(), checks });
    let mut failures = Vec::new();
    let mut sink = |failure| failures.push(failure);
    let (active, _) = collect_integer_requirement_failures_in_scope(
        calls, IntegerRequirementEntry::Arbitrary, None, false, false, &mut sink,
        OrderedRootLiteralProofs::<STEP_FIRST> { conditions: OrderedRootConditions(body), observed: &observed },
    )?;
    Some((active, failures, observed.into_inner().returned))
}

#[cfg(feature = "internal-checker")]
pub(super) fn collect_ordered_root_multiple_proof<'a, 'check, 'bound>(
    calls: &'a FunctionCallSites, body: &'a super::ScalarBlock, select_witness: &'check mut SymbolicProofSelect<'bound>,
) -> Option<(bool, Vec<IntegerCheckFailure<'a>>, bool, Option<SymbolicLiteralFailure<'a>>)> {
    let observed = std::cell::RefCell::new(MultipleProofEvidence { select_witness, returned: false, failure: None });
    let mut failures = Vec::new();
    let mut sink = |failure| failures.push(failure);
    let (active, _) = collect_integer_requirement_failures_in_scope(
        calls, IntegerRequirementEntry::Arbitrary, None, false, false, &mut sink,
        OrderedRootMultipleProof { conditions: OrderedRootConditions(body), observed: &observed },
    )?;
    let observed = observed.into_inner();
    Some((active, failures, observed.returned, observed.failure))
}

fn collect_integer_requirement_failures_in_scope<'a, Scoped: ScopedAuthoredReturn<'a>>(
    calls: &'a FunctionCallSites,
    entry: IntegerRequirementEntry<'_>,
    handle_call: Option<&mut ActualCallHandler<'a, '_>>,
    allow_projected_calls: bool,
    allow_contextual_local_transfers: bool,
    sink: &mut dyn FnMut(IntegerCheckFailure<'a>),
    scope: Scoped,
) -> Option<(bool, IntegerRequirementContinuation)> {
    #[cfg(feature = "internal-checker")]
    let ordinary = Scoped::ALLOW_ORDINARY_INTEGER_LOCALS;
    #[cfg(not(feature = "internal-checker"))]
    let ordinary = false;
    let (tail, exact, projected) = match entry {
        IntegerRequirementEntry::Arbitrary => (ordinary_tail_type(calls, ordinary), None, None),
        IntegerRequirementEntry::Exact(arguments) => (None, Some(arguments), None),
        IntegerRequirementEntry::Projected { slot, values, fixed } => {
            let row = match (slot, fixed) {
                (0, None) => vec![values],
                (0, Some(fixed)) => vec![values, Rc::new(IntegerSet::singleton(fixed))],
                (1, Some(fixed)) => vec![Rc::new(IntegerSet::singleton(fixed)), values],
                _ => return None,
            };
            (None, None, Some((slot, IntegerRow::new(row))))
        }
    };
    let mut collected = IntegerFailureCollection {
        continuation: IntegerRequirementContinuation::Empty, projected, allow_projected_calls, allow_contextual_local_transfers, sink,
    };
    let ProofCoverage::Applicable(active) = run_integer_callable::<false, true, _>(
        calls, tail, None, None, &mut [], exact, handle_call, Some(&mut collected), false, scope,
    )?.ok()? else { return None; };
    Some((active, collected.continuation))
}

pub(super) type ExactIntegerArgument<'a> = (&'a ScalarType, &'a BigInt);

pub(super) fn exact_typed_call_argument_at<'a>(
    structure: &'a OccurrenceStructure, call: OccurrenceOrdinal, slot: usize, allow_bool: bool,
    lexical: impl FnOnce(OccurrenceOrdinal, &ScalarType) -> Option<&'a BigInt>,
) -> Option<ExactIntegerArgument<'a>> {
    let (argument, ty) = checked_scalar_call_argument_at(structure, call, slot, allow_bool)?;
    let value = scalar_call_literal(structure, argument, ty, allow_bool).or_else(|| lexical(argument, ty))?;
    Some((ty, value))
}

fn checked_scalar_call_argument_at(
    structure: &OccurrenceStructure, call: OccurrenceOrdinal, slot: usize, allow_bool: bool,
) -> Option<(OccurrenceOrdinal, &ScalarType)> {
    let OccurrenceNode::Expression(argument) = structure.edges.get(&(
        OccurrenceParent::Expression(call), OccurrenceEdge::CallArgument(slot),
    ))? else {
        return None;
    };
    let ty = structure.checked_types.expressions.get(argument)?;
    (is_integer_type(ty) || (allow_bool && *ty == ScalarType::Bool)).then_some((*argument, ty))
}

fn scalar_call_literal<'a>(structure: &'a OccurrenceStructure, argument: OccurrenceOrdinal, ty: &ScalarType, ordinary: bool)
    -> Option<&'a BigInt>
{
    #[cfg(feature = "internal-checker")]
    if *ty == ScalarType::Bool {
        return structure.checker_ordinary_bool_literals.get(&argument);
    }
    let common = structure.checker_integer_literals.get(&argument);
    #[cfg(feature = "internal-checker")]
    if ordinary && common.is_none() {
        return structure.checker_ordinary_call_integer_literals.get(&argument);
    }
    #[cfg(not(feature = "internal-checker"))]
    let _ = (ty, ordinary);
    common
}

fn ordinary_parameter_set(ty: &ScalarType, allow_bool: bool) -> Option<IntegerSet> {
    #[cfg(feature = "internal-checker")]
    if allow_bool && *ty == ScalarType::Bool {
        return Some(IntegerSet::ordinary_bool_domain());
    }
    #[cfg(not(feature = "internal-checker"))]
    let _ = allow_bool;
    IntegerSet::from_type(ty)
}

pub(super) enum ActualCallResult {
    Unit,
    EmptyUnit,
    ProjectedUnit(Rc<IntegerSet>),
    ProjectedUnitPair(ProjectedUnitPair),
    Integer(BigInt),
    PartitionedInteger(Vec<IntegerReturnPartition>),
    PartitionedIntegerPair(ProjectedIntegerPair),
}

pub(super) struct ProjectedUnitPair {
    caller_axes: [usize; 2],
    survivors: IntegerInputs,
}

pub(super) struct ProjectedIntegerPair {
    caller_axes: [usize; 2],
    pieces: Vec<IntegerPairReturnPartition>,
}

struct IntegerPairReturnPartition {
    anchor: u16,
    inputs: [Rc<IntegerSet>; 2],
    output: IntegerPairPartitionOutput,
}

enum ComputedIntegerReturn {
    Projected(IntegerReturnPartition),
    Pair(IntegerPairReturnPartition),
}

enum IntegerPairPartitionOutput {
    Singleton(Rc<IntegerSet>),
    InputAlias(usize),
    InputAffine(usize, Rc<IntegerAffineTransform>),
}

pub(super) const MAX_ACTUAL_CALL_DEPTH: usize = 64;
const MAX_PARTITION_TRANSFER_WORK: usize = MAX_ACTUAL_CALL_DEPTH * MAX_ACTUAL_CALL_DEPTH;

pub(super) struct IntegerReturnPartition {
    span: super::ByteSpan,
    input: Rc<IntegerSet>,
    output: IntegerPartitionOutput,
}

enum IntegerPartitionOutput {
    Singleton(Rc<IntegerSet>),
    InputAlias,
    InputAffine(Rc<IntegerAffineTransform>),
}

#[cfg(feature = "internal-checker")]
pub(super) struct PartitionedCallProof<'a> {
    pub(super) applicable: bool,
    pub(super) covers_guard: bool,
    pub(super) failure: Option<SymbolicLiteralFailure<'a>>,
}

#[cfg(feature = "internal-checker")]
pub(super) enum PartitionedProofGoal<'a> {
    Literal(IntegerRelation, &'a BigInt),
    EntryEquality,
}

#[cfg(feature = "internal-checker")]
pub(super) fn verify_partitioned_call_return<'a>(
    producer: &'a FunctionCallSites, returned: &ActualCallResult, domain: &IntegerSet,
    eligible_for_elision: bool, goal: PartitionedProofGoal<'_>,
) -> Option<PartitionedCallProof<'a>> {
    let ActualCallResult::PartitionedInteger(pieces) = returned else { return None; };
    let [binding] = producer.boundary.checker_parameter_bindings.as_deref()? else { return None; };
    let [parameter] = producer.boundary.parameters.as_slice() else { return None; };
    let mut applicable = false;
    let mut covered = Vec::new();
    for piece in pieces {
        let output = match (&goal, &piece.output) {
            (_, IntegerPartitionOutput::InputAffine(_))
            | (PartitionedProofGoal::Literal(..), IntegerPartitionOutput::InputAlias) => return None,
            (_, IntegerPartitionOutput::InputAlias) => None,
            (_, IntegerPartitionOutput::Singleton(output)) => Some(output.singleton_value()?),
        };
        let input = match piece.input.intersection(domain) {
            IntegerIntersection::Unchanged => Rc::clone(&piece.input),
            IntegerIntersection::Empty => continue,
            IntegerIntersection::Changed(clipped) => Rc::new(clipped),
        };
        applicable = true;
        let failure = match (&goal, output) {
            (PartitionedProofGoal::Literal(relation, bound), Some(value)) if !relation.accepts(value.cmp(bound)) =>
                Some((Rc::clone(&input), value)),
            (PartitionedProofGoal::EntryEquality, Some(value)) if input.singleton_value() != Some(value) => {
                let unequal = IntegerRelation::from_operator(&BinaryOperator::NotEqual)?;
                Some((Rc::new(input.keep_relation(unequal, value)), value))
            }
            _ => None,
        };
        if let Some((violating_inputs, value)) = failure {
            return Some(PartitionedCallProof { applicable: true, covers_guard: false, failure: Some(SymbolicLiteralFailure {
                output: piece.span, returned: value.clone(), state: IntegerInputs::from_cells(vec![vec![violating_inputs]]),
                axes: vec![(binding, &parameter.ty)],
            }) });
        }
        if eligible_for_elision {
            covered.push(input);
        }
    }
    let covers_guard = eligible_for_elision && applicable
        && domain.is_subset_of(&IntegerSet::union(covered.iter().map(Rc::as_ref)));
    Some(PartitionedCallProof { applicable, covers_guard, failure: None })
}

#[derive(Clone, Copy)]
pub(super) enum ProjectedCompanion<'a> {
    Unary,
    Fixed(ExactIntegerArgument<'a>),
    Equal,
    Independent { ty: &'a ScalarType, axis: usize },
}

pub(super) struct ProjectedIntegerArgument<'a> {
    pub(super) ty: &'a ScalarType,
    pub(super) caller_axis: usize,
    pub(super) slot: usize,
    path: &'a IntegerInputs,
    pub(super) companion: ProjectedCompanion<'a>,
    affine_work: Option<&'a mut usize>,
}

#[cfg(test)]
std::thread_local! {
    pub(super) static PROJECTED_ARGUMENT_MATERIALIZATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl ProjectedIntegerArgument<'_> {
    pub(super) fn materialize_bounded(&mut self) -> Option<Rc<IntegerSet>> {
        let Some(work) = self.affine_work.as_deref_mut() else { return self.materialize(); };
        #[cfg(test)]
        PROJECTED_ARGUMENT_MATERIALIZATIONS.with(|count| count.set(count.get() + 1));
        let (first, rest) = self.path.0.split_first()?;
        let first = &first[self.caller_axis];
        charge_call_work(work, 1)?;
        let mut remaining = rest.iter().map(|row| &row[self.caller_axis]);
        while let Some(domain) = remaining.next() {
            charge_call_work(work, 1)?;
            if Rc::ptr_eq(first, domain) { continue; }

            // Only allocate the distinct-set table once the shared-domain fast
            // path fails. Prefix rows all point to `first`; don't rescan them.
            let mut seen = std::collections::HashSet::new();
            let mut unique = Vec::new();
            let mut intervals = 0usize;
            for domain in [first, domain].into_iter().chain(remaining) {
                charge_call_work(work, 1)?;
                if seen.insert(Rc::as_ptr(domain)) {
                    let count = domain.interval_count();
                    charge_call_work(work, count)?;
                    intervals = intervals.checked_add(count)?;
                    unique.push(domain.as_ref());
                }
            }
            // IntegerSet::union gathers, sorts and normalizes intervals.
            let levels = usize::BITS - intervals.leading_zeros();
            charge_call_work(work, intervals.checked_mul(1 + levels as usize)?)?;
            return Some(Rc::new(IntegerSet::union(unique)));
        }
        Some(Rc::clone(first))
    }

    pub(super) fn materialize(&self) -> Option<Rc<IntegerSet>> {
        #[cfg(test)]
        PROJECTED_ARGUMENT_MATERIALIZATIONS.with(|count| count.set(count.get() + 1));
        self.path.project_axis(self.caller_axis)
    }
}

#[test]
fn bounded_projection_charges_shared_domain_by_row_not_interval() {
    let shared = Rc::new(IntegerSet::union((0..8).map(|value| IntegerSet::singleton(&BigInt::from(value * 2)))
        .collect::<Vec<_>>().iter()));
    let rows = IntegerInputs((0..16).map(|_| IntegerRow::new(vec![Rc::clone(&shared)])).collect());
    let mut work = MAX_PARTITION_TRANSFER_WORK - 16;
    let mut projection = ProjectedIntegerArgument {
        ty: &ScalarType::I8, caller_axis: 0, slot: 0, path: &rows,
        companion: ProjectedCompanion::Unary, affine_work: Some(&mut work),
    };
    assert!(Rc::ptr_eq(&projection.materialize_bounded().unwrap(), &shared));
    assert_eq!(work, MAX_PARTITION_TRANSFER_WORK);
}

#[test]
fn bounded_projection_charges_distinct_interval_sort_before_materializing() {
    let first = Rc::new(IntegerSet::singleton(&BigInt::from(1)));
    let second = Rc::new(IntegerSet::singleton(&BigInt::from(3)));
    let rows = IntegerInputs(vec![IntegerRow::new(vec![first]), IntegerRow::new(vec![second])]);
    let mut work = MAX_PARTITION_TRANSFER_WORK - 9;
    let mut projection = ProjectedIntegerArgument {
        ty: &ScalarType::I8, caller_axis: 0, slot: 0, path: &rows,
        companion: ProjectedCompanion::Unary, affine_work: Some(&mut work),
    };
    assert!(projection.materialize_bounded().is_none());
}

pub(super) enum NestedIntegerArgument<'a> {
    Single(Option<ExactIntegerArgument<'a>>),
    #[cfg(feature = "internal-checker")]
    RootLexical(ExactIntegerArgument<'a>),
    #[cfg(feature = "internal-checker")]
    ContextualValue(ExactIntegerArgument<'a>),
    Pair(ExactIntegerArgument<'a>, ExactIntegerArgument<'a>),
    Projected(ProjectedIntegerArgument<'a>),
}

enum NestedIntegerOperand<'a> {
    Exact(ExactIntegerArgument<'a>),
    Axis(&'a ScalarType, usize),
}

// Hold a sticky exhausted value so argument discovery can distinguish a
// missing operand from a refused scan even on the unary fallback route.
fn charge_call_work(work: &mut usize, cost: usize) -> Option<()> {
    *work = work.saturating_add(cost);
    (*work <= MAX_PARTITION_TRANSFER_WORK).then_some(())
}

fn nested_integer_operand<'a>(
    calls: &'a FunctionCallSites, expression: OccurrenceOrdinal, slot: usize,
    roots: &IntegerRoots<'_>, path: &'a IntegerInputs, allow_bool: bool, work: Option<&mut usize>,
) -> Option<NestedIntegerOperand<'a>> {
    let (argument, ty) = checked_scalar_call_argument_at(&calls.structure, expression, slot, allow_bool)?;
    if let Some(value) = scalar_call_literal(&calls.structure, argument, ty, allow_bool) {
        return Some(NestedIntegerOperand::Exact((ty, value)));
    }
    let PlainNameIdentity::Lexical(binding) = calls.prepared_plain_name(argument)? else { return None; };
    let (axis, binding_type) = roots.lookup(binding)?;
    if binding_type != ty {
        return None;
    }
    let singleton = if let Some(work) = work {
        path.singleton_axis_with(axis, || charge_call_work(work, 1)).map(|(value, _)| value)
    } else {
        path.singleton_at(axis)
    };
    Some(match singleton {
        Some(value) => NestedIntegerOperand::Exact((ty, value)),
        None => NestedIntegerOperand::Axis(ty, axis),
    })
}

fn nested_pair_argument<'a>(
    calls: &'a FunctionCallSites, expression: OccurrenceOrdinal, roots: &IntegerRoots<'_>, path: &'a IntegerInputs,
    allow_projection: bool, allow_equal: bool, allow_independent: bool, allow_bool: bool, mut work: Option<&mut usize>,
) -> Option<NestedIntegerArgument<'a>> {
    let first = nested_integer_operand(calls, expression, 0, roots, path, allow_bool, work.as_deref_mut())?;
    let second = nested_integer_operand(calls, expression, 1, roots, path, allow_bool, work.as_deref_mut())?;
    let mut equal_axes = |left, right| {
        left == right || path.0.iter().all(|row| {
            if let Some(work) = work.as_deref_mut() {
                // One binary search per canonical lookup, not a scan of all aliases.
                let comparisons = usize::BITS - row.equal_axes().len().leading_zeros();
                if charge_call_work(work, 1 + 2 * comparisons as usize).is_none() { return false; }
            }
            row.canonical_axis(left) == row.canonical_axis(right)
        })
    };
    let (ty, caller_axis, slot, companion) = match (first, second) {
        (NestedIntegerOperand::Exact(first), NestedIntegerOperand::Exact(second)) =>
            return Some(NestedIntegerArgument::Pair(first, second)),
        (NestedIntegerOperand::Axis(ty, axis), NestedIntegerOperand::Exact(fixed)) if allow_projection =>
            (ty, axis, 0, ProjectedCompanion::Fixed(fixed)),
        (NestedIntegerOperand::Exact(fixed), NestedIntegerOperand::Axis(ty, axis)) if allow_projection =>
            (ty, axis, 1, ProjectedCompanion::Fixed(fixed)),
        (NestedIntegerOperand::Axis(left_ty, left), NestedIntegerOperand::Axis(right_ty, right))
            if allow_equal && left_ty == right_ty && equal_axes(left, right) =>
            (left_ty, left, 0, ProjectedCompanion::Equal),
        (NestedIntegerOperand::Axis(ty, left), NestedIntegerOperand::Axis(right_ty, right)) if allow_independent =>
            (ty, left, 0, ProjectedCompanion::Independent { ty: right_ty, axis: right }),
        _ => return None,
    };
    Some(NestedIntegerArgument::Projected(ProjectedIntegerArgument { ty, caller_axis, slot, path, companion, affine_work: None }))
}

fn actual_nested_argument<'a>(
    calls: &'a FunctionCallSites,
    expression: OccurrenceOrdinal,
    roots: &IntegerRoots<'_>,
    path: &'a IntegerInputs,
    allow_projection: bool, allow_bool: bool, work: Option<&mut usize>,
) -> NestedIntegerArgument<'a> {
    match nested_integer_operand(calls, expression, 0, roots, path, allow_bool, work) {
        Some(NestedIntegerOperand::Exact(argument)) => NestedIntegerArgument::Single(Some(argument)),
        Some(NestedIntegerOperand::Axis(ty, caller_axis)) if allow_projection =>
            NestedIntegerArgument::Projected(ProjectedIntegerArgument {
                ty, caller_axis, slot: 0, path, companion: ProjectedCompanion::Unary, affine_work: None,
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
    run_integer_callable::<false, false, _>(calls, tail, None, None, &mut [], exact_arguments, Some(handle_call), None, false, ())
        .map(|result| result.map(|coverage| matches!(coverage, ProofCoverage::Applicable(true))))
}

pub(super) fn proves_requirement_with_imported_root_local<'a>(
    calls: &'a FunctionCallSites,
    local: ImportedLocal<'a>,
) -> Option<bool> {
    if matches!(&local.values, ImportedIntegerValues::Ready(values, _) if values.is_empty()) {
        return None;
    }
    match run_integer_callable::<true, false, _>(calls, integer_tail_type(calls), None, None, &mut [Some(local)], None, None, None, false, ())? {
        Ok(ProofCoverage::Applicable(true)) => Some(true),
        Ok(ProofCoverage::Applicable(false)) => None,
        Ok(ProofCoverage::NoApplicableInputs) => None,
        Err(_) => Some(false),
    }
}

#[cfg(feature = "internal-checker")]
pub(super) fn collect_ordered_imported_root_locals<'a>(
    calls: &'a FunctionCallSites, imports: &mut [Option<ImportedLocal<'a>>], sink: &mut dyn FnMut(IntegerCheckFailure<'a>),
) -> Option<bool> {
    let mut failures = IntegerFailureCollection {
        continuation: IntegerRequirementContinuation::Empty, projected: None, allow_projected_calls: false, allow_contextual_local_transfers: false, sink,
    };
    let ProofCoverage::Applicable(active) = run_integer_callable::<true, true, _>(
        calls, integer_tail_type(calls), None, None, imports, None, None, Some(&mut failures), false, (),
    )?.ok()? else { return None; };
    Some(active)
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
    match run_integer_callable::<false, false, _>(calls, Some(tail), entry_filter, Some(&mut paths), &mut [], exact_arguments, None, None, proof_reflexive_branches, authored_return)? {
        Ok(ProofCoverage::Applicable(_)) if !paths.is_empty() => {
            Some(Ok(ProofCoverage::Applicable(paths)))
        }
        Ok(ProofCoverage::Applicable(_)) => None,
        Ok(ProofCoverage::NoApplicableInputs) => Some(Ok(ProofCoverage::NoApplicableInputs)),
        Err(failure) => Some(Err(failure)),
    }
}

#[cfg(feature = "internal-checker")]
pub(super) fn collect_exact_integer_return_requirements<'a>(
    calls: &'a FunctionCallSites, expected: &ScalarType, arguments: &[&BigInt],
    sink: &mut dyn FnMut(IntegerCheckFailure<'a>),
) -> Option<(bool, BigInt)> {
    #[derive(Clone, Copy)]
    struct ExactReturn<'a>(&'a std::cell::RefCell<Option<BigInt>>);

    impl<'body> ScopedAuthoredReturn<'body> for ExactReturn<'_> {
        const ENABLED: bool = false;
        const OBSERVE_RETURN: bool = true;
        const REQUIRE_TOTAL_RETURN: bool = true;

        fn at(
            self, _: &FunctionCallSites, _: BlockOrdinal, _: FinalOutputSlot, _: &ScalarType,
            _: &IntegerRoots<'_>, _: &IntegerInputs,
        ) -> Option<&'body BigInt> {
            None
        }

        fn observe_return(
            self, _: FinalOutputSlot, value: IntegerReturn<'body>, roots: &IntegerRoots<'body>, path: &IntegerInputs,
        ) -> Option<()> {
            let value = match value {
                IntegerReturn::Const(value) => value,
                IntegerReturn::Alias(binding) => path.singleton_at(roots.lookup(binding)?.0)?,
            };
            let mut returned = self.0.borrow_mut();
            if let Some(earlier) = returned.as_ref() {
                return (earlier == value).then_some(());
            }
            *returned = Some(value.clone());
            Some(())
        }
    }

    let tail = integer_tail_type(calls).filter(|tail| *tail == expected)?;
    let mut failures = IntegerFailureCollection {
        continuation: IntegerRequirementContinuation::Empty, projected: None, allow_projected_calls: false, allow_contextual_local_transfers: false, sink,
    };
    let returned = std::cell::RefCell::new(None);
    let ProofCoverage::Applicable(active) = run_integer_callable::<false, true, _>(
        calls, Some(tail), None, None, &mut [], Some(arguments), None,
        Some(&mut failures), false, ExactReturn(&returned),
    )?.ok()? else { return None; };
    Some((active, returned.into_inner()?))
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
    let (first, rest) = paths.split_first()?;
    let value = first.singleton_return()?;
    rest.iter().all(|path| path.singleton_return() == Some(value)).then(|| value.clone())
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
    branches: &'a [std::cell::OnceCell<Option<BigInt>>; 2],
) -> Option<Result<ProofCoverage<Vec<IntegerReturningPath<'a>>>, IntegerCheckFailure<'a>>> {
    observe_integer_returns_in_scope(calls, entry_filter, None, false, NamedAuthoredReturn { block: calls.structure.root, body, value, branches: Some(branches) })
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

fn ordinary_tail_type(calls: &FunctionCallSites, allow_bool: bool) -> Option<&ScalarType> {
    #[cfg(feature = "internal-checker")]
    if allow_bool {
        let results = &calls.boundary.results;
        if let ([result], [output]) = (results.as_slice(), calls.boundary.root_outputs.as_slice()) {
            if result.ty == ScalarType::Bool && output.ty == result.ty { return Some(&result.ty); }
        }
    }
    #[cfg(not(feature = "internal-checker"))]
    let _ = allow_bool;
    integer_tail_type(calls)
}


fn run_integer_callable<'a, const USE_IMPORTED_BOUND: bool, const COLLECT: bool, Authored: ScopedAuthoredReturn<'a>>(
    calls: &'a FunctionCallSites,
    tail: Option<&'a ScalarType>,
    entry_filter: Option<&[CheckerPredicate]>,
    returns: Option<&mut Vec<IntegerReturningPath<'a>>>,
    mut imports: &mut [Option<ImportedLocal<'a>>],
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
        proof_reflexive_branches,
        #[cfg(feature = "internal-checker")]
        allow_ordinary_integer_locals: Authored::ALLOW_ORDINARY_INTEGER_LOCALS,
        #[cfg(feature = "internal-checker")]
        allow_ordinary_affine_overwrites: Authored::ALLOW_ORDINARY_INTEGER_LOCALS,
        #[cfg(feature = "internal-checker")]
        ordinary_comparison_work: (Authored::ALLOW_ORDINARY_INTEGER_LOCALS
            && calls.structure.has_ordinary_predicates()).then_some(0),
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
        exact_integer_entry_row(arguments)
    } else {
        #[cfg(feature = "internal-checker")]
        let allow_bool = Authored::ALLOW_ORDINARY_INTEGER_LOCALS;
        #[cfg(not(feature = "internal-checker"))]
        let allow_bool = false;
        boundary.parameters.iter()
            .map(|parameter| ordinary_parameter_set(&parameter.ty, allow_bool).map(Rc::new))
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
    let result = walk_integer_block::<USE_IMPORTED_BOUND, COLLECT, _>(calls, &mut roots, calls.structure.root, IntegerInputs(vec![row]), tail, returns, &mut handle_call, &mut failures, &mut imports, authored_return)?;
    #[cfg(feature = "internal-checker")]
    if Authored::REQUIRE_TOTAL_RETURN && !matches!(result, IntegerControl::Returned) {
        return None;
    }
    if USE_IMPORTED_BOUND && !imports.is_empty() {
        #[cfg(feature = "internal-checker")]
        let skipped_shared = COLLECT
            && matches!(&result, IntegerControl::Continue(path) if path.is_unreachable())
            && imports.iter().all(|import| matches!(import, Some(ImportedLocal { values: ImportedIntegerValues::Shared { .. }, .. })));
        #[cfg(not(feature = "internal-checker"))]
        let skipped_shared = false;
        if !skipped_shared {
            return None;
        }
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
            IntegerControl::PendingCall(_) => return None,
        };
    }
    match result {
        IntegerControl::Continue(_) | IntegerControl::Returned => {
            Some(Ok(ProofCoverage::Applicable(roots.reached_obligation)))
        }
        IntegerControl::Fail(failure) => Some(Err(failure)),
        IntegerControl::PendingCall(_) => None,
    }
}

#[cfg(all(test, feature = "internal-checker"))]
thread_local! {
    pub(super) static DIRECTIVE_DISJUNCTION_NORMALIZATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn directive_disjunction_cuts<'a>(
    [(lhs, left), (rhs, right)]: &'a [(IntegerRelation, BigInt); 2],
    (minimum, maximum): (&'a BigInt, &'a BigInt),
) -> Option<IntegerCutUnion<'a>> {
    let fits = |bound: &BigInt| minimum <= bound && bound <= maximum;
    if !fits(left) || !fits(right) {
        return None;
    }
    #[cfg(all(test, feature = "internal-checker"))]
    DIRECTIVE_DISJUNCTION_NORMALIZATIONS.with(|count| count.set(count.get() + 1));
    Some(IntegerCutUnion::new((*lhs, left), (*rhs, right), (minimum, maximum)))
}

#[cfg(feature = "internal-checker")]
enum OrdinaryComparisonCut<'a> {
    Constant(bool),
    Independent { axes: [usize; 2], relation: IntegerRelation },
    Bound { axis: usize, relation: IntegerRelation, bound: Cow<'a, BigInt> },
}

#[cfg(feature = "internal-checker")]
fn ordinary_comparison_cut(
    row: &IntegerRow, [left, right]: [usize; 2], relation: IntegerRelation,
) -> Option<OrdinaryComparisonCut<'_>> {
    use OrdinaryComparisonCut::{Bound, Constant};
    use std::cmp::Ordering::{Equal, Greater, Less};
    let left = row.canonical_axis(left);
    let right = row.canonical_axis(right);
    if left == right { return Some(Constant(relation.accepts(Equal))); }
    match (row[left].singleton_value(), row[right].singleton_value()) {
        (Some(l), Some(r)) => return Some(Constant(relation.accepts(l.cmp(r)))),
        (None, Some(r)) => return Some(Bound { axis: left, relation, bound: Cow::Borrowed(r) }),
        (Some(l), None) => return Some(Bound { axis: right, relation: relation.reversed(), bound: Cow::Borrowed(l) }),
        (None, None) => {}
    }

    let Some(class) = row.affine.as_deref().filter(|class| class.contains(left) && class.contains(right)) else {
        return Some(OrdinaryComparisonCut::Independent { axes: [left, right], relation });
    };
    let zero = BigInt::default();
    let parts = |axis| {
        if axis == class.anchor { Some((false, &zero)) }
        else { let member = class.member(axis)?; Some((member.reflected, &member.offset)) }
    };
    let (left_reflected, left_offset) = parts(left)?;
    let (right_reflected, right_offset) = parts(right)?;
    if left_reflected == right_reflected {
        return Some(Constant(relation.accepts(left_offset.cmp(right_offset))));
    }
    // Put the positive slope on the left: 2*anchor R difference.
    let (relation, difference) = if left_reflected {
        (relation.reversed(), left_offset - right_offset)
    } else {
        (relation, right_offset - left_offset)
    };
    let mut bound = &difference / 2u8;
    let odd = (&difference % 2u8).bits() != 0;
    let relation = if odd {
        // Floor rather than truncate for a negative odd difference.
        if difference.sign() == num_bigint::Sign::Minus { bound -= 1u8; }
        match (relation.accepts(Less), relation.accepts(Greater)) {
            (same, other) if same == other => return Some(Constant(same)),
            (true, false) => IntegerRelation::from_operator(&BinaryOperator::LessEqual)?,
            (false, true) => IntegerRelation::from_operator(&BinaryOperator::Greater)?,
            _ => return None,
        }
    } else { relation };
    Some(Bound { axis: class.anchor, relation, bound: Cow::Owned(bound) })
}

#[cfg(feature = "internal-checker")]
const MAX_INDEPENDENT_COMPARISON_POINTS: usize = 16;

#[cfg(feature = "internal-checker")]
fn comparison_domain_size(values: &IntegerSet, max: usize, work: &mut usize) -> Option<Option<usize>> {
    let mut size = 0usize;
    for (first, last) in values.intervals() {
        charge_call_work(work, 1)?;
        let width = (last - first) + 1u8;
        if width > BigInt::from(max.checked_sub(size)?) { return Some(None); }
        size = size.checked_add(usize::try_from(width).ok()?)?;
    }
    Some(Some(size))
}

#[cfg(feature = "internal-checker")]
fn independent_uniform(
    left: &IntegerSet, right: &IntegerSet, relation: IntegerRelation, work: &mut usize,
) -> Option<Option<bool>> {
    use std::cmp::Ordering::{Equal, Greater, Less};
    let (left_min, left_max) = left.extremes()?;
    let (right_min, right_max) = right.extremes()?;
    let less = left_min < right_max;
    let greater = left_max > right_min;
    let unequal = match (less, greater) {
        (true, true) if relation.accepts(Less) != relation.accepts(Greater) => return Some(None),
        (true, _) => relation.accepts(Less),
        (_, true) => relation.accepts(Greater),
        (false, false) => return Some(Some(relation.accepts(Equal))),
    };
    if unequal == relation.accepts(Equal) || left_max < right_min || right_max < left_min {
        return Some(Some(unequal));
    }

    // Equality can still be impossible inside overlapping extrema, for
    // instance when each input occupies different points of a hole.
    let mut lhs = left.intervals();
    let mut rhs = right.intervals();
    let (mut a, mut b) = (lhs.next(), rhs.next());
    while let (Some((af, al)), Some((bf, bl))) = (a, b) {
        charge_call_work(work, 1)?;
        if al < bf { a = lhs.next(); }
        else if bl < af { b = rhs.next(); }
        else { return Some(None); }
    }
    Some(Some(unequal))
}

#[cfg(feature = "internal-checker")]
fn prepared_comparison_axes(
    roots: &IntegerRoots<'_>, comparison: &super::reference::CheckerOrdinaryIntegerComparison,
) -> Option<[usize; 2]> {
    let (left, left_ty) = roots.lookup(&comparison.operands[0])?;
    let (right, right_ty) = roots.lookup(&comparison.operands[1])?;
    (left_ty == &comparison.ty && right_ty == &comparison.ty).then_some([left, right])
}

#[cfg(feature = "internal-checker")]
enum OrdinaryIntegerAtom<'a> {
    Lexical([usize; 2], IntegerRelation),
    Literal(usize, IntegerRelation, &'a BigInt),
}

#[cfg(feature = "internal-checker")]
impl IntegerInputs {
    fn ordinary_comparison(
        &self, axes: [usize; 2], relation: IntegerRelation, selected: [bool; 2], work: &mut usize,
    ) -> Option<[Self; 2]> {
        self.ordinary_atom(OrdinaryIntegerAtom::Lexical(axes, relation), selected, work)
    }

    fn ordinary_atom(&self, atom: OrdinaryIntegerAtom<'_>, selected: [bool; 2], work: &mut usize) -> Option<[Self; 2]> {
        let mut output = [Self::default(), Self::default()];
        for row in &self.0 {
            let cut = match atom {
                OrdinaryIntegerAtom::Lexical(axes, relation) => {
                    // Bound identity/singleton discovery and constant-width
                    // arithmetic before deriving the checked integer cut.
                    let lookups = usize::BITS - row.equal_axes().len().leading_zeros();
                    charge_call_work(work, 8 + 2 * lookups as usize)?;
                    ordinary_comparison_cut(row, axes, relation)?
                }
                OrdinaryIntegerAtom::Literal(axis, relation, bound) => {
                    charge_call_work(work, 1)?;
                    OrdinaryComparisonCut::Bound { axis, relation, bound: Cow::Borrowed(bound) }
                }
            };
            match cut {
                OrdinaryComparisonCut::Constant(result) => {
                    let lane = result as usize;
                    if selected[lane] {
                        charge_call_work(work, row.len().checked_add(1)?)?;
                        output[lane].0.push(row.clone());
                    }
                }
                OrdinaryComparisonCut::Bound { axis, relation, bound } =>
                    Self::emit_ordinary_bound(row, axis, relation, &bound, None, selected, work, &mut output)?,
                OrdinaryComparisonCut::Independent { axes, relation } => {
                    if let Some(uniform) = independent_uniform(&row[axes[0]], &row[axes[1]], relation, work)? {
                        let lane = uniform as usize;
                        if selected[lane] {
                            charge_call_work(work, row.len().checked_add(1)?)?;
                            output[lane].0.push(row.clone());
                        }
                    } else {
                        Self::emit_independent_comparison(row, axes, relation, selected, work, &mut output)?;
                    }
                }
            }
        }
        Some(output)
    }

    fn emit_independent_comparison(
        row: &IntegerRow, [left, right]: [usize; 2], relation: IntegerRelation,
        selected: [bool; 2], work: &mut usize, output: &mut [Self; 2],
    ) -> Option<()> {
        let left_size = comparison_domain_size(&row[left], MAX_INDEPENDENT_COMPARISON_POINTS, work)?;
        let right_limit = left_size.map_or(MAX_INDEPENDENT_COMPARISON_POINTS, |size| size.saturating_sub(1));
        let right_size = comparison_domain_size(&row[right], right_limit, work)?;
        // Prefer source-left on a tie; enumerate only the smaller axis,
        // in ascending order and never over unproved or oversized ranges.
        let (small, broad, relation, count) = if let Some(size) = right_size {
            (right, left, relation, size)
        } else {
            (left, right, relation.reversed(), left_size?)
        };
        charge_call_work(work, count.checked_add(row[small].interval_count())?)?;
        for (first, last) in row[small].intervals() {
            let mut point = first.clone();
            loop {
                Self::emit_ordinary_bound(row, broad, relation, &point, Some(small), selected, work, output)?;
                if &point == last { break; }
                point += 1u8;
            }
        }
        Some(())
    }

    fn emit_ordinary_bound(
        row: &IntegerRow, axis: usize, relation: IntegerRelation, bound: &BigInt, fixed: Option<usize>,
        selected: [bool; 2], work: &mut usize, output: &mut [Self; 2],
    ) -> Option<()> {
        let intervals = row[axis].interval_count();
        charge_call_work(work, intervals.checked_mul(3)?.checked_add(5)?)?;
        let subsets = match selected {
            [true, true] => {
                let (yes, no) = row[axis].split_relation(relation, bound);
                [Some(no), Some(yes)]
            }
            [true, false] => [Some(row[axis].keep_relation(relation.complement(), bound)), None],
            [false, true] => [None, Some(row[axis].keep_relation(relation, bound))],
            [false, false] => [None, None],
        };
        for (lane, narrowed) in subsets.into_iter().enumerate() {
            let Some(narrowed) = narrowed else { continue; };
            if narrowed.is_empty() { continue; }
            charge_call_work(work, row.len().checked_add(1)?)?;
            let changed = narrowed != *row[axis];
            if changed {
                charge_call_work(work, row.equal_axes().len())?;
                row.charge_affine_replacement(axis, narrowed.interval_count(), work)?;
            }
            if let Some(fixed) = fixed {
                charge_call_work(work, row.equal_axes().len().checked_add(1)?)?;
                row.charge_affine_replacement(fixed, 1, work)?;
            }
            if *work > MAX_PARTITION_TRANSFER_WORK { return None; }
            let mut retained = row.clone();
            if changed { retained.replace_domain(axis, Rc::new(narrowed)); }
            if let Some(fixed) = fixed {
                // The selected singleton belongs to the incoming typed cell;
                // the two axes have already been proven independent in this row.
                retained.replace_domain(fixed, Rc::new(IntegerSet::singleton(bound)));
            }
            output[lane].0.push(retained);
        }
        Some(())
    }

    fn ordinary_boolean(
        &self, condition: &super::reference::CheckerOrdinaryBoolean, roots: &IntegerRoots<'_>,
        selected: [bool; 2], work: &mut usize,
    ) -> Option<[Self; 2]> {
        use super::reference::CheckerOrdinaryBoolean as Boolean;
        let mut output = [Self::default(), Self::default()];
        if self.is_unreachable() { return Some(output); }
        charge_call_work(work, 1)?;
        match condition {
            Boolean::Opaque | Boolean::Call(_) => return None,
            Boolean::Constant(value) => {
                let lane = *value as usize;
                if selected[lane] {
                    for row in &self.0 {
                        charge_call_work(work, row.len().checked_add(1)?)?;
                    }
                    output[lane].0 = self.0.clone();
                }
            }
            Boolean::Binding(binding) => {
                let (axis, ty) = roots.lookup(binding)?;
                if *ty != ScalarType::Bool { return None; }
                let one = BigInt::from(1u8);
                let relation = IntegerRelation::from_operator(&BinaryOperator::Equal)?;
                return self.ordinary_atom(OrdinaryIntegerAtom::Literal(axis, relation, &one), selected, work);
            }
            Boolean::Integer(predicate) => {
                let (axis, ty) = roots.lookup(&predicate.binding)?;
                if ty != &predicate.ty { return None; }
                let relation = IntegerRelation::from_operator(&predicate.operator)?;
                return self.ordinary_atom(OrdinaryIntegerAtom::Literal(axis, relation, &predicate.bound), selected, work);
            }
            Boolean::Lexical(comparison) => {
                let axes = prepared_comparison_axes(roots, comparison)?;
                return self.ordinary_comparison(axes, comparison.relation, selected, work);
            }
            Boolean::Not(value) => {
                let [no, yes] = self.ordinary_boolean(value, roots, [selected[1], selected[0]], work)?;
                return Some([yes, no]);
            }
            Boolean::And(operands) | Boolean::Or(operands) => {
                let and = matches!(condition, Boolean::And(_));
                let continuing = usize::from(and);
                let stop = 1 - continuing;
                let mut left_selected = selected;
                left_selected[continuing] = true;
                let mut left = self.ordinary_boolean(&operands[0], roots, left_selected, work)?;
                let right = left[continuing].ordinary_boolean(&operands[1], roots, selected, work)?;
                output[stop] = std::mem::take(&mut left[stop]);
                for (output, mut right) in output.iter_mut().zip(right) {
                    let pushed = right.0.len();
                    if pushed != 0 {
                        let total = output.0.len().checked_add(pushed)?;
                        let relocated = if total > output.0.capacity() { output.0.len() } else { 0 };
                        charge_call_work(work, pushed.checked_add(relocated)?)?;
                        output.0.append(&mut right.0);
                    }
                }
            }
        }
        Some(output)
    }

    fn ordinary_predicate_at(
        &self, structure: &OccurrenceStructure, condition: OccurrenceOrdinal, roots: &mut IntegerRoots<'_>,
        selected: [bool; 2],
    ) -> Option<Option<[Self; 2]>> {
        let Some(mut work) = roots.ordinary_comparison_work else { return Some(None); };
        let partitions = if let Some(compound) = structure.checker_ordinary_booleans.get(&condition) {
            self.ordinary_boolean(compound, roots, selected, &mut work)?
        } else if let Some(comparison) = structure.checker_ordinary_integer_comparisons.get(&condition) {
            let axes = prepared_comparison_axes(roots, comparison)?;
            self.ordinary_comparison(axes, comparison.relation, selected, &mut work)?
        } else {
            return Some(None);
        };
        roots.ordinary_comparison_work = Some(work);
        Some(Some(partitions))
    }

}

struct UnitBranchFrame {
    block: BlockOrdinal,
    next: usize,
    depth: usize,
    axis: Option<usize>,
    keep_arm_rows: bool,
    alternate: Option<(Option<BlockOrdinal>, IntegerInputs)>,
    first: Option<IntegerInputs>,
}

#[cfg(feature = "internal-checker")]
enum GuardFrame {
    Negate,
    EvaluateRight { continuing: usize, right: super::reference::CheckerOrdinaryBoolean },
    Merge { stop: usize, short: IntegerInputs },
}

#[cfg(feature = "internal-checker")]
enum GuardProgress {
    Call(OccurrenceOrdinal),
    Done([IntegerInputs; 2]),
}

#[cfg(feature = "internal-checker")]
struct OrdinaryGuard {
    condition: OccurrenceOrdinal,
    next: Option<super::reference::CheckerOrdinaryBoolean>,
    frames: Vec<GuardFrame>,
    returned: Option<[IntegerInputs; 2]>,
}

#[cfg(feature = "internal-checker")]
impl OrdinaryGuard {
    fn advance(&mut self, path: &mut IntegerInputs, roots: &mut IntegerRoots<'_>) -> Option<GuardProgress> {
        use super::reference::CheckerOrdinaryBoolean as Boolean;
        let mut work = roots.ordinary_comparison_work?;
        loop {
            charge_call_work(&mut work, 1)?;
            if let Some(node) = self.next.take() {
                if path.is_unreachable() {
                    self.returned = Some([IntegerInputs::default(), IntegerInputs::default()]);
                } else {
                    match node {
                        Boolean::Call(call) => {
                            roots.ordinary_comparison_work = Some(work);
                            return Some(GuardProgress::Call(call));
                        }
                        Boolean::Not(value) => {
                            self.frames.push(GuardFrame::Negate);
                            self.next = Some(*value);
                        }
                        Boolean::And(operands) => {
                            let [left, right] = *operands;
                            self.frames.push(GuardFrame::EvaluateRight { continuing: 1, right });
                            self.next = Some(left);
                        }
                        Boolean::Or(operands) => {
                            let [left, right] = *operands;
                            self.frames.push(GuardFrame::EvaluateRight { continuing: 0, right });
                            self.next = Some(left);
                        }
                        leaf => {
                            self.returned = Some(path.ordinary_boolean(&leaf, roots, [true, true], &mut work)?);
                            *path = IntegerInputs::default();
                        }
                    }
                }
                continue;
            }
            let mut rows = self.returned.take()?;
            match self.frames.pop() {
                Some(GuardFrame::Negate) => {
                    rows.swap(0, 1);
                    self.returned = Some(rows);
                }
                Some(GuardFrame::EvaluateRight { continuing, right }) => {
                    *path = std::mem::take(&mut rows[continuing]);
                    // Do not resolve or activate any leaf in a dead RHS.
                    if path.is_unreachable() {
                        self.returned = Some(rows);
                    } else {
                        let stop = 1 - continuing;
                        self.frames.push(GuardFrame::Merge { stop, short: std::mem::take(&mut rows[stop]) });
                        self.next = Some(right);
                    }
                }
                Some(GuardFrame::Merge { stop, mut short }) => {
                    let pushed = rows[stop].0.len();
                    if pushed != 0 {
                        let total = short.0.len().checked_add(pushed)?;
                        let relocated = if total > short.0.capacity() { short.0.len() } else { 0 };
                        charge_call_work(&mut work, pushed.checked_add(relocated)?)?;
                        short.0.append(&mut rows[stop].0);
                    }
                    rows[stop] = short;
                    self.returned = Some(rows);
                }
                None => {
                    roots.ordinary_comparison_work = Some(work);
                    return Some(GuardProgress::Done(rows));
                }
            }
        }
    }
}

struct IntegerBlockCursor {
    block: BlockOrdinal,
    next: usize,
    depth: usize,
    path: IntegerInputs,
    branches: Vec<UnitBranchFrame>,
    terminal: Vec<(BlockOrdinal, IntegerInputs, usize)>,
    #[cfg(feature = "internal-checker")]
    guard: Option<OrdinaryGuard>,
}

impl IntegerBlockCursor {
    fn new(block: BlockOrdinal, depth: usize, path: IntegerInputs) -> Self {
        Self { block, next: 0, depth, path, branches: Vec::new(), terminal: Vec::new(),
            #[cfg(feature = "internal-checker")]
            guard: None,
        }
    }
}

fn walk_integer_block<'a, const USE_IMPORTED_BOUND: bool, const COLLECT: bool, Authored: ScopedAuthoredReturn<'a>>(
    calls: &'a FunctionCallSites,
    roots: &mut IntegerRoots<'a>,
    block: BlockOrdinal,
    path: IntegerInputs,
    root_tail: Option<&'a ScalarType>,
    returns: Option<&mut Vec<IntegerReturningPath<'a>>>,
    handle_call: &mut Option<&mut ActualCallHandler<'a, '_>>,
    failures: &mut Option<&mut IntegerFailureCollection<'a, '_>>,
    imports: &mut &mut [Option<ImportedLocal<'a>>],
    authored_return: Authored,
) -> Option<IntegerControl<'a>> {
    let mut cursor = IntegerBlockCursor::new(block, roots.axes.len(), path);
    drive_integer_block::<USE_IMPORTED_BOUND, COLLECT, _, false>(
        calls, roots, &mut cursor, root_tail, returns, handle_call, failures, imports, authored_return, false, None, None, None,
    )
}

fn drive_integer_block<'a, const USE_IMPORTED_BOUND: bool, const COLLECT: bool, Authored: ScopedAuthoredReturn<'a>, const SUSPEND_CALL: bool>(
    calls: &'a FunctionCallSites,
    roots: &mut IntegerRoots<'a>,
    cursor: &mut IntegerBlockCursor,
    root_tail: Option<&'a ScalarType>,
    mut returns: Option<&mut Vec<IntegerReturningPath<'a>>>,
    handle_call: &mut Option<&mut ActualCallHandler<'a, '_>>,
    failures: &mut Option<&mut IntegerFailureCollection<'a, '_>>,
    imports: &mut &mut [Option<ImportedLocal<'a>>],
    authored_return: Authored,
    suspend_integer: bool,
    mut return_capture_work: Option<&mut usize>,
    #[allow(unused_mut)] mut computed_returns: Option<&mut Vec<(usize, ComputedIntegerReturn)>>,
    mut discarded_return: Option<&mut bool>,
) -> Option<IntegerControl<'a>> {
    let IntegerBlockCursor { mut block, mut next, mut depth, mut path, mut branches, mut terminal,
        #[cfg(feature = "internal-checker")]
        mut guard,
    } =
        std::mem::replace(cursor, IntegerBlockCursor::new(calls.structure.root, 0, IntegerInputs::default()));
    let structure = &calls.structure;
    loop {
        if let Some(work) = &mut roots.unit_pair_work {
            let axes = roots.axes.len().checked_add(1)?;
            *work = work.checked_add(1)?.checked_add(path.0.len().checked_mul(axes)?)?;
            if *work > MAX_PARTITION_TRANSFER_WORK { return None; }
        }
        if let Some(work) = &mut roots.affine_work {
            for row in &path.0 {
                *work = work.checked_add(1)?;
                if let Some(relation) = row.affine.as_deref() {
                    // A single prepared cut can split one interval. Charge both pieces, both
                    // directions of transfer, and equality-group propagation up front.
                    let intervals = row[relation.anchor].interval_count().checked_add(1)?;
                    let aliases = row.equal_axes().len().checked_add(1)?;
                    *work = work.checked_add(intervals.checked_add(aliases)?.checked_mul(4)?.checked_mul(relation.members.len())?)?;
                }
                if *work > MAX_PARTITION_TRANSFER_WORK { return None; }
            }
        }
        let items = &structure.blocks.get(block.0)?.items;
        #[cfg(feature = "internal-checker")]
        let guard_pending = guard.is_some();
        #[cfg(not(feature = "internal-checker"))]
        let guard_pending = false;
        let Some(item) = items.get(next).filter(|_| !path.is_unreachable() || guard_pending) else {
            if SUSPEND_CALL && root_tail.is_some() && branches.is_empty() && path.is_unreachable() {
                if let Some((arm, incoming, arm_depth)) = terminal.pop() {
                    roots.truncate(arm_depth);
                    block = arm;
                    path = incoming;
                    next = 0;
                    continue;
                }
                if returns.as_ref().is_some_and(|paths| !paths.is_empty())
                    || computed_returns.as_ref().is_some_and(|pieces| !pieces.is_empty())
                    || discarded_return.as_deref().copied() == Some(true)
                {
                    return Some(IntegerControl::Returned);
                }
            }
            if roots.axes.len() != depth {
                path.drop_locals(depth);
                roots.truncate(depth);
            }
            let Some(mut frame) = branches.pop() else {
                return Some(IntegerControl::Continue(path));
            };
            if let Some((alternate_block, alternate_path)) = frame.alternate.take() {
                if let Some(alternate_block) = alternate_block {
                    frame.first = Some(path);
                    branches.push(frame);
                    block = alternate_block;
                    next = 0;
                    depth = roots.axes.len();
                    path = alternate_path;
                    continue;
                }
                #[cfg(feature = "internal-checker")]
                if !frame.keep_arm_rows && !path.is_unreachable() && !alternate_path.is_unreachable() {
                    path.charge_local_alias_work(&mut roots.ordinary_local_work, 2)?;
                    alternate_path.charge_local_alias_work(&mut roots.ordinary_local_work, 2)?;
                }
                if frame.keep_arm_rows {
                    path.0.extend(alternate_path.0);
                } else {
                    path = path.join_equal(alternate_path, frame.axis);
                }
            } else if let Some(mut first) = frame.first {
                #[cfg(feature = "internal-checker")]
                if !frame.keep_arm_rows && !path.is_unreachable() && !first.is_unreachable() {
                    path.charge_local_alias_work(&mut roots.ordinary_local_work, 2)?;
                    first.charge_local_alias_work(&mut roots.ordinary_local_work, 2)?;
                }
                if frame.keep_arm_rows {
                    first.0.extend(path.0);
                    path = first;
                } else {
                    path = first.join_equal(path, frame.axis);
                }
            }
            block = frame.block;
            next = frame.next;
            depth = frame.depth;
            continue;
        };
        next += 1;
        let tail = if branches.is_empty() { root_tail } else { None };
        #[cfg(feature = "internal-checker")]
        let evaluated_guard = {
            let mut result = None;
            if SUSPEND_CALL && suspend_integer && roots.ordinary_comparison_work.is_some()
                && !structure.checker_ordinary_call_guards.is_empty()
            {
                let condition = match &item.kind {
                    BlockItemKind::Evaluation { expression } => match calls.conditionals.get(expression) {
                        Some(ConditionalRegion::Unit { condition, .. }) => Some(*condition),
                        Some(ConditionalRegion::Value { condition, .. })
                            if structure.checked_types.expressions.get(expression) == Some(&ScalarType::Unit) =>
                        {
                            Some(*condition)
                        }
                        _ => None,
                    },
                    BlockItemKind::FinalOutput(slot) if tail.is_some() => match calls.conditionals.get(&slot.expression) {
                        Some(ConditionalRegion::Value { condition, .. }) => Some(*condition),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some((condition, nodes)) = condition.and_then(|condition|
                    structure.checker_ordinary_call_guards.get(&condition).map(|nodes| (condition, *nodes)))
                {
                    if guard.is_none() {
                        charge_call_work(roots.ordinary_comparison_work.as_mut()?, nodes)?;
                        let boolean = structure.checker_ordinary_booleans.get(&condition)?.clone();
                        guard = Some(OrdinaryGuard {
                            condition, next: Some(boolean), frames: Vec::new(), returned: None,
                        });
                    }
                    let active = guard.as_mut()?;
                    if active.condition != condition { return None; }
                    match active.advance(&mut path, roots)? {
                        GuardProgress::Call(call) => {
                            *cursor = IntegerBlockCursor {
                                block, next: next.checked_sub(1)?, depth, path, branches, terminal, guard,
                            };
                            return Some(IntegerControl::PendingCall(ExactCallEffect::Scalar {
                                call, binding: None, ty: &ScalarType::Bool,
                            }));
                        }
                        GuardProgress::Done(rows) => { result = Some(rows); guard = None; }
                    }
                } else if guard.is_some() { return None; }
            }
            result
        };
        match item.kind {
            BlockItemKind::Evaluation { expression } => {
                if let Some(directive) = structure.checker_directives.get(&expression) {
                    #[cfg(feature = "internal-checker")]
                    if COLLECT && roots.ordinary_comparison_work.is_some() {
                        let require = directive.operation == CheckerOperation::Require;
                        if let Some([rejected, accepted]) = path.ordinary_predicate_at(
                            structure, expression, roots, [require, !require],
                        )? {
                            roots.reached_obligation = true;
                            let failing = if require { rejected } else if accepted.is_unreachable() {
                                std::mem::take(&mut path)
                            } else {
                                path = accepted;
                                IntegerInputs::default()
                            };
                            if !failing.is_unreachable() {
                                let work = roots.ordinary_comparison_work.as_mut()?;
                                let equal_axes = failing.common_equal_axes_bounded(work)?;
                                charge_call_work(work, roots.axes.len())?;
                                (failures.as_deref_mut()?.sink)(IntegerCheckFailure {
                                    directive: expression, equal_axes, state: failing, axes: roots.axes.clone(),
                                });
                            }
                            continue;
                        }
                    }
                    let predicate = directive.predicate.as_ref()?;
                    roots.reached_obligation = true;
                    #[cfg(feature = "internal-checker")]
                    if matches!(predicate, CheckerPredicate::Integer(_) | CheckerPredicate::DirectiveDisjunction { .. }) {
                        path.charge_local_alias_work(&mut roots.ordinary_local_work, 2)?;
                    }
                    if let (true, CheckerPredicate::DirectiveDisjunction { binding, ty, atoms }) = (COLLECT, predicate) {
                        let (axis, actual_type) = roots.lookup(binding)?;
                        if ty != actual_type {
                            return None;
                        }
                        let (minimum, maximum) = super::typecheck::integer_bounds(ty)?;
                        let cuts = directive_disjunction_cuts(atoms, (&minimum, &maximum))?;
                        if let Some(work) = &mut roots.affine_work {
                            path.charge_affine_disjunction(axis, &cuts, directive.operation == CheckerOperation::Require, work)?;
                        }
                        let failing = match directive.operation {
                            CheckerOperation::Require => path.disjunction_failure_witness(axis, &cuts),
                            CheckerOperation::Narrow => path.narrow_disjunction_or_witness(axis, &cuts),
                        };
                        if !failing.is_unreachable() {
                            (failures.as_deref_mut()?.sink)(IntegerCheckFailure {
                                directive: expression, equal_axes: failing.common_equal_axes(), state: failing, axes: roots.axes.clone(),
                            });
                        }
                        continue;
                    }
                    if COLLECT && directive.operation == CheckerOperation::Require {
                        let failing = path.failing_witness(roots, predicate)?;
                        if !failing.is_unreachable() {
                            (failures.as_deref_mut()?.sink)(IntegerCheckFailure {
                                directive: expression, equal_axes: failing.common_equal_axes(), state: failing, axes: roots.axes.clone(),
                            });
                        }
                        continue;
                    }
                    let (passing, failing, _) = path.split(roots, predicate)?;
                    if directive.operation.rejects(!passing.is_unreachable(), !failing.is_unreachable()) {
                        let failure = IntegerCheckFailure {
                            directive: expression, equal_axes: failing.common_equal_axes(), state: failing, axes: roots.axes.clone(),
                        };
                        if COLLECT {
                            (failures.as_deref_mut()?.sink)(failure);
                        } else {
                            return Some(IntegerControl::Fail(failure));
                        }
                    }
                    path = passing;
                } else if structure.checker_direct_targets.contains_key(&expression) {
                    if SUSPEND_CALL {
                        *cursor = IntegerBlockCursor { block, next, depth, path, branches, terminal, #[cfg(feature = "internal-checker")] guard };
                        return Some(IntegerControl::PendingCall(ExactCallEffect::Unit(expression)));
                    }
                    let handle = handle_call.as_mut()?;
                    let signature = calls.checked_call_signatures.get(&expression)?;
                    let project = COLLECT && failures.as_ref().is_some_and(|collection| collection.allow_projected_calls);
                    let argument = if COLLECT && signature.parameters.len() == 2 {
                        nested_pair_argument(calls, expression, roots, &path, project, false, false, false, None)?
                    } else {
                        actual_nested_argument(calls, expression, roots, &path, project, false, None)
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
                    #[cfg(feature = "internal-checker")]
                    let ordinary = {
                        let comparison = if evaluated_guard.is_some() {
                            evaluated_guard
                        } else if authored_return.prefer_legacy_root_compound(calls, block, *condition, roots) {
                            None
                        } else {
                            path.ordinary_predicate_at(structure, *condition, roots, [true, true])?
                        };
                        if comparison.is_none()
                            && !matches!(structure.checker_branches.get(condition), Some(CheckerPredicate::Boolean(_)))
                        {
                            path.charge_local_alias_work(&mut roots.ordinary_local_work, 4)?;
                        }
                        comparison
                    };
                    #[cfg(not(feature = "internal-checker"))]
                    let ordinary: Option<[IntegerInputs; 2]> = None;
                    let keep_arm_rows = ordinary.is_some();
                    let (yes, no, axis) = if let Some([no, yes]) = ordinary {
                        (yes, no, None)
                    } else if let Some(predicate) = structure.checker_branches.get(condition) {
                        path.split(roots, predicate)?
                    } else {
                        #[cfg(feature = "internal-checker")]
                        {
                            authored_return.split_unprepared(calls, RootBranchSlot {
                                block, item: item.position, parent: expression, condition: *condition, output: None,
                            }, roots, path)?
                        }
                        #[cfg(not(feature = "internal-checker"))]
                        return None;
                    };
                    let (arm, incoming, alternate) = if yes.is_unreachable() {
                        let Some(else_block) = else_block.filter(|_| !no.is_unreachable()) else {
                            path = no;
                            continue;
                        };
                        (else_block, no, None)
                    } else {
                        (then_block, yes, (!no.is_unreachable()).then_some((else_block, no)))
                    };
                    branches.push(UnitBranchFrame { block, next, depth, axis, keep_arm_rows, alternate, first: None });
                    block = arm;
                    next = 0;
                    depth = roots.axes.len();
                    path = incoming;
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
                    #[cfg(feature = "internal-checker")]
                    let ordinary = {
                        let comparison = if evaluated_guard.is_some() {
                            evaluated_guard
                        } else if authored_return.prefer_legacy_root_compound(calls, block, *condition, roots) {
                            None
                        } else {
                            path.ordinary_predicate_at(structure, *condition, roots, [true, true])?
                        };
                        if comparison.is_none()
                            && !matches!(structure.checker_branches.get(condition), Some(CheckerPredicate::Boolean(_)))
                        {
                            path.charge_local_alias_work(&mut roots.ordinary_local_work, 4)?;
                        }
                        comparison
                    };
                    #[cfg(not(feature = "internal-checker"))]
                    let ordinary: Option<[IntegerInputs; 2]> = None;
                    let (yes, no, _) = if let Some([no, yes]) = ordinary {
                        (yes, no, None)
                    } else if let Some(predicate) = structure.checker_branches.get(condition) {
                        path.split(roots, predicate)?
                    } else {
                        #[cfg(feature = "internal-checker")]
                        {
                            authored_return.split_unprepared(calls, RootBranchSlot {
                                block, item: slot.item, parent: slot.expression, condition: *condition, output: Some(slot.output),
                            }, roots, path)?
                        }
                        #[cfg(not(feature = "internal-checker"))]
                        return None;
                    };
                    if SUSPEND_CALL {
                        if !branches.is_empty() {
                            return None;
                        }
                        (block, path) = match (yes.is_unreachable(), no.is_unreachable()) {
                            (false, false) => {
                                terminal.push((*else_block, no, roots.axes.len()));
                                (*then_block, yes)
                            }
                            (false, true) => (*then_block, yes),
                            (true, false) => (*else_block, no),
                            _ => return None,
                        };
                        next = 0;
                        continue;
                    }
                    for (index, (arm, incoming)) in [(*then_block, yes), (*else_block, no)].into_iter().enumerate() {
                        if incoming.is_unreachable() {
                            continue;
                        }
                        let branch_return = if Authored::ENABLED {
                            authored_return.branch(structure, block, slot, index, arm)
                        } else {
                            authored_return
                        };
                        match walk_integer_block::<USE_IMPORTED_BOUND, COLLECT, _>(calls, roots, arm, incoming, Some(expected), returns.as_deref_mut(), handle_call, failures, imports, branch_return)? {
                            IntegerControl::Returned => {}
                            failure @ IntegerControl::Fail(_) => return Some(failure),
                            IntegerControl::Continue(outgoing) if COLLECT && outgoing.is_unreachable() => {
                                #[cfg(feature = "internal-checker")]
                                if Authored::REQUIRE_TOTAL_RETURN {
                                    return None;
                                }
                            }
                            IntegerControl::Continue(_) => return None,
                            IntegerControl::PendingCall(_) => return None,
                        }
                    }
                    roots.truncate(depth);
                    return Some(IntegerControl::Returned);
                }
                #[cfg(feature = "internal-checker")]
                if let Some(target) = computed_returns.as_deref_mut() {
                    if let Some((operator, operands)) = structure.checker_integer_final_operations.get(&slot.expression) {
                        let work = return_capture_work.as_deref_mut()?;
                        charge_partition_capture(work, &path, roots.axes.len().checked_add(1)?)?;
                        let at = returns.as_ref()?.len();
                        let operation = AuthoredIntegerOperation::checked(operator)?;
                        let (min, max) = super::typecheck::integer_bounds(ty)?;
                        let axis = |operand| match calls.prepared_plain_name(operand)? {
                            PlainNameIdentity::Lexical(binding) => {
                                let (axis, actual) = roots.lookup(binding)?;
                                (actual == ty).then_some(axis)
                            }
                            _ => None,
                        };
                        let [left, right] = operands.map(|operand| {
                            (structure.checker_integer_literals.get(&operand), axis(operand))
                        });
                        for row in &path.0 {
                            let value = computed_projected_value(row, roots.projected_entry_axis,
                                operation, left, right, &min, &max)?;
                            let piece = if let Some(formal) = roots.projected_entry_axis {
                                ComputedIntegerReturn::Projected(IntegerReturnPartition {
                                    span: slot.span, input: Rc::clone(row.entry_domain(formal)),
                                    output: match value {
                                        ComputedProjectedValue::Singleton(output) => IntegerPartitionOutput::Singleton(output),
                                        ComputedProjectedValue::Identity(_) => IntegerPartitionOutput::InputAlias,
                                        ComputedProjectedValue::Affine(_, transform) => IntegerPartitionOutput::InputAffine(transform),
                                    },
                                })
                            } else {
                                ComputedIntegerReturn::Pair(IntegerPairReturnPartition {
                                    anchor: row.projected_from,
                                    inputs: [Rc::clone(&row[0]), Rc::clone(&row[1])],
                                    output: match value {
                                        ComputedProjectedValue::Singleton(output) => IntegerPairPartitionOutput::Singleton(output),
                                        ComputedProjectedValue::Identity(axis) => IntegerPairPartitionOutput::InputAlias(axis),
                                        ComputedProjectedValue::Affine(axis, transform) => IntegerPairPartitionOutput::InputAffine(axis, transform),
                                    },
                                })
                            };
                            target.push((at, piece));
                        }
                        path = IntegerInputs::default();
                        continue;
                    }
                }
                let value = if let Some(value) = scalar_call_literal(structure, slot.expression, ty, false) {
                    IntegerReturn::Const(value)
                } else if let Some(PlainNameIdentity::Lexical(binding)) = calls.prepared_plain_name(slot.expression) {
                    let (_, binding_type) = roots.lookup(binding)?;
                    if binding_type != ty {
                        return None;
                    }
                    IntegerReturn::Alias(binding)
                } else if Authored::ENABLED && tail.is_some() {
                    IntegerReturn::Const(authored_return.at(calls, block, slot, ty, roots, &path)?)
                } else {
                    return None;
                };
                if tail.is_some() {
                    if let Some(complete) = discarded_return.as_deref_mut() { *complete = true; }
                    #[cfg(feature = "internal-checker")]
                    if Authored::OBSERVE_RETURN {
                        authored_return.observe_return(slot, value, roots, &path)?;
                    }
                    if let Some(returns) = returns.as_deref_mut() {
                        if let Some(work) = return_capture_work.as_deref_mut() {
                            charge_partition_capture(work, &path, roots.axes.len())?;
                        }
                        returns.push(IntegerReturningPath {
                            span: slot.span,
                            value,
                            ty,
                            state: path,
                            axes: roots.axes.iter().map(|(binding, _)| *binding).collect(),
                        });
                    }
                    if SUSPEND_CALL {
                        path = IntegerInputs::default();
                        continue;
                    }
                    roots.truncate(depth);
                    return Some(IntegerControl::Returned);
                }
            }
            BlockItemKind::Assignment { group } => {
                let allow_contextual_local_transfers = || COLLECT && !USE_IMPORTED_BOUND
                    && failures.as_deref().is_some_and(|failure| failure.allow_contextual_local_transfers);
                let (axis, value, rhs, destination) = integer_assignment(calls, roots, group, &path, allow_contextual_local_transfers)?;
                #[cfg(not(feature = "internal-checker"))]
                let _ = (rhs, destination);
                let preserve_entry = roots.projected_entry_axis == Some(axis);
                if let Some(work) = &mut roots.affine_work {
                    if path.0.iter().any(|row| row.affine_touches(axis)) {
                        #[cfg(feature = "internal-checker")]
                        {
                            if !roots.allow_ordinary_affine_overwrites
                                || (axis < roots.parameters && roots.unit_pair_work.is_some())
                                || !structure.checker_integer_literals.contains_key(&rhs)
                                || structure.checker_contextual_occurrences.contains(&destination)
                            {
                                return None;
                            }
                            path.overwrite_affine(axis, &value, preserve_entry, work)?;
                            continue;
                        }
                        #[cfg(not(feature = "internal-checker"))]
                        { let _ = work; return None; }
                    }
                }
                if axis < roots.parameters && !preserve_entry && path.axis_has_alias(axis) {
                    return None;
                }
                #[cfg(feature = "internal-checker")]
                path.charge_local_alias_work(&mut roots.ordinary_local_work, 2)?;
                path = path.overwrite(axis, value, preserve_entry);
            }
            BlockItemKind::Binding(ref binding) => {
                if USE_IMPORTED_BOUND && block == structure.root
                    && imports.first().and_then(Option::as_ref).is_some_and(|local| local.position == item.position)
                {
                    let (local, rest) = std::mem::take(imports).split_first_mut()?;
                    *imports = rest;
                    let local = local.take()?;
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
                            CheckerIntegerCallArgument::ContextualValue => return None,
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
                    let applies = |guard: &ImportedEntryGuard<'_>| match arguments.get(guard.slot).copied().flatten() {
                        Some(Argument::Literal(value)) => guard.relation.accepts(value.cmp(guard.bound)),
                        Some(Argument::Axis(axis)) => path.0.iter().all(|row| row[axis].every(guard.relation, guard.bound)),
                        None => false,
                    };
                    if !local.guards.iter().all(&applies) {
                        return None;
                    }
                    #[cfg(feature = "internal-checker")]
                    let in_domain = |(slot, domain): (usize, &IntegerSet)| match arguments.get(slot).copied().flatten() {
                        Some(Argument::Literal(value)) => domain.contains(value),
                        Some(Argument::Axis(axis)) => path.0.iter().all(|row| row[axis].is_subset_of(domain)),
                        None => false,
                    };
                    let values = match local.values {
                        ImportedIntegerValues::Ready(values, _) => values,
                        #[cfg(feature = "internal-checker")]
                        ImportedIntegerValues::Shared { values, bound, guards, domain } => {
                            if !guards.iter().all(applies) || domain.is_some_and(|domain| !in_domain(domain)) {
                                return None;
                            }
                            roots.add(&origin.binding, &receiver.ty);
                            let values = values.get_or_init(|| Rc::new(IntegerSet::singleton(bound)));
                            path.append_shared_local(values);
                            continue;
                        }
                        #[cfg(feature = "internal-checker")]
                        ImportedIntegerValues::Deferred { slot, domain, issue } => {
                            let admitted = !path.is_unreachable() && in_domain((slot, domain));
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
                    let [receiver] = binding.receivers() else { return None; };
                    if SUSPEND_CALL && suspend_integer && !matches!(origin.arguments.as_slice(), [(_, CheckerIntegerCallArgument::ContextualValue)]) {
                        *cursor = IntegerBlockCursor { block, next, depth, path, branches, terminal, #[cfg(feature = "internal-checker")] guard };
                        return Some(IntegerControl::PendingCall(ExactCallEffect::Scalar {
                            call: origin.call, binding: Some(&origin.binding), ty: &receiver.ty,
                        }));
                    }
                    let handle = handle_call.as_mut()?;
                    let signature = calls.checked_call_signatures.get(&origin.call)?;
                    let argument = actual_nested_argument(calls, origin.call, roots, &path, false, false, None);
                    #[cfg(feature = "internal-checker")]
                    let argument = if matches!(origin.arguments.as_slice(), [(_, CheckerIntegerCallArgument::ContextualValue)]) {
                        let NestedIntegerArgument::Single(Some(exact)) = argument else { return None; };
                        NestedIntegerArgument::ContextualValue(exact)
                    } else {
                        argument
                    };
                    #[cfg(not(feature = "internal-checker"))]
                    if matches!(origin.arguments.as_slice(), [(_, CheckerIntegerCallArgument::ContextualValue)]) {
                        return None;
                    }
                    #[cfg(feature = "internal-checker")]
                    let argument = match argument {
                        NestedIntegerArgument::Single(Some(exact)) if COLLECT && block == structure.root
                            && structure.checker_integer_call_locals.len() > 1
                            && matches!(origin.arguments.as_slice(), [(_, CheckerIntegerCallArgument::PlainName)]) =>
                        {
                            NestedIntegerArgument::RootLexical(exact)
                        }
                        argument => argument,
                    };
                    let ActualCallResult::Integer(value) = handle(calls, origin.call, signature, argument, &receiver.ty)? else {
                        return None;
                    };
                    roots.add(&origin.binding, &receiver.ty);
                    path.append_local(IntegerSet::singleton(&value));
                    continue;
                }
                if let [receiver] = binding.receivers() {
                    if let Some(identity) = &receiver.contextual_storage {
                        if !super::typecheck::is_integer_type(&receiver.ty) {
                            return None;
                        }
                        let BindingInitialization::Outputs(outputs) = &binding.initialization else { return None; };
                        let value = if let Some(literal) = structure.checker_integer_literals.get(&outputs.primary) {
                            Rc::new(IntegerSet::singleton(literal))
                        } else {
                            if !structure.checker_contextual_occurrences.contains(&outputs.primary) {
                                return None;
                            }
                            let PlainNameIdentity::Lexical(source) = calls.prepared_plain_name(outputs.primary)? else { return None; };
                            let (axis, ty) = roots.lookup(source)?;
                            if ty != &receiver.ty {
                                return None;
                            }
                            Rc::clone(path.singleton_axis(axis)?.1)
                        };
                        let identity = structure.checker_integer_locals.get(&(block, item.position))
                            .filter(|prepared| *prepared == identity)?;
                        roots.add(identity, &receiver.ty);
                        path.append_shared_local(&value);
                        continue;
                    }
                }
                #[cfg(feature = "internal-checker")]
                if roots.allow_ordinary_integer_locals {
                    if let Some(origin) = structure.checker_ordinary_scalar_locals.get(&(block, item.position)) {
                        let [receiver] = binding.receivers() else { return None; };
                        let BindingInitialization::Outputs(outputs) = &binding.initialization else { return None; };
                        if origin.source != outputs.primary || !outputs.secondary.is_empty()
                            || receiver.contextual_storage.is_some()
                            || structure.checked_types.expressions.get(&origin.source)? != &receiver.ty
                        {
                            return None;
                        }
                        match &origin.kind {
                            super::reference::CheckerOrdinaryScalarLocalKind::Copy => {
                                let PlainNameIdentity::Lexical(source) = calls.prepared_plain_name(origin.source)? else { return None; };
                                let (axis, ty) = roots.lookup(source)?;
                                if ty != &receiver.ty { return None; }
                                // Read the old binding before installing the new one.
                                for row in &path.0 {
                                    let work = if row.affine.is_some() {
                                        roots.affine_work.as_mut()?
                                    } else {
                                        roots.ordinary_local_work.get_or_insert(0)
                                    };
                                    charge_call_work(work, row.equal_axes().len().checked_add(2)?)?;
                                }
                                for row in &mut path.0 {
                                    row.append_alias(axis, Rc::clone(&row[axis]));
                                }
                            }
                            super::reference::CheckerOrdinaryScalarLocalKind::BoolCall(call) => {
                                if receiver.ty != ScalarType::Bool || !SUSPEND_CALL || !suspend_integer { return None; }
                                *cursor = IntegerBlockCursor { block, next, depth, path, branches, terminal, #[cfg(feature = "internal-checker")] guard };
                                return Some(IntegerControl::PendingCall(ExactCallEffect::Scalar {
                                    call: *call, binding: Some(&origin.binding), ty: &receiver.ty,
                                }));
                            }
                            super::reference::CheckerOrdinaryScalarLocalKind::BoolLiteral(value) => {
                                if receiver.ty != ScalarType::Bool { return None; }
                                let work = roots.ordinary_local_work.get_or_insert(0);
                                charge_call_work(work, path.0.len().checked_add(1)?)?;
                                path.append_local(IntegerSet::singleton(&BigInt::from(u8::from(*value))));
                            }
                            super::reference::CheckerOrdinaryScalarLocalKind::Arithmetic { operator, operands } => {
                                let operation = AuthoredIntegerOperation::checked(operator)?;
                                let operand = |operand| ordinary_local_operand(calls, roots, &receiver.ty, operand);
                                let left = operand(&operands[0])?;
                                let right = operand(&operands[1])?;
                                path.append_ordinary_calculation(operation, left, right, &receiver.ty,
                                    &mut roots.ordinary_local_work, &mut roots.affine_work)?;
                            }
                        }
                        roots.add(&origin.binding, &receiver.ty);
                        continue;
                    }
                }
                if let Some((identity, ty, value)) = integer_local(calls, binding, block, item.position) {
                    roots.add(identity, ty);
                    path.append_local(IntegerSet::singleton(value));
                } else {
                    if !COLLECT || USE_IMPORTED_BOUND
                        || !failures.as_deref().is_some_and(|failure| failure.allow_contextual_local_transfers)
                    {
                        return None;
                    }
                    let identity = structure.checker_contextual_read_locals.get(&(block, item.position))?;
                    let [receiver] = binding.receivers() else { return None; };
                    let BindingInitialization::Outputs(outputs) = &binding.initialization else { return None; };
                    let PlainNameIdentity::Lexical(source) = calls.prepared_plain_name(outputs.primary)? else { return None; };
                    let (axis, ty) = roots.lookup(source)?;
                    if ty != &receiver.ty {
                        return None;
                    }
                    let value = Rc::clone(path.singleton_axis(axis)?.1);
                    roots.add(identity, &receiver.ty);
                    path.append_shared_local(&value);
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum ExactCallEffect<'a> {
    Unit(OccurrenceOrdinal),
    Scalar { call: OccurrenceOrdinal, binding: Option<&'a ReferenceBindingId>, ty: &'a ScalarType },
}

impl ExactCallEffect<'_> {
    pub(super) fn occurrence(self) -> OccurrenceOrdinal {
        match self {
            Self::Unit(call) | Self::Scalar { call, .. } => call,
        }
    }

    pub(super) fn expected(&self) -> &ScalarType {
        match self {
            Self::Unit(_) => &ScalarType::Unit,
            Self::Scalar { ty, .. } => ty,
        }
    }
}

pub(super) enum ExactCallProgress<'a> {
    Pending(ExactCallEffect<'a>),
    Done { active: bool, result: ActualCallResult },
    Failed(IntegerCheckFailure<'a>),
}

fn charge_partition_capture(retained: &mut usize, path: &IntegerInputs, axes: usize) -> Option<()> {
    let mut work = retained.checked_add(axes)?.checked_add(1)?;
    for row in &path.0 {
        work = work.checked_add(1)?.checked_add(row.len())?.checked_add(row.equal_axes().len())?;
        if let Some(ProjectedEntry::Detached(domain)) = row.entry.as_deref() {
            work = work.checked_add(domain.interval_count())?;
        }
        for domain in row.iter() {
            work = work.checked_add(domain.interval_count())?;
            if work > MAX_PARTITION_TRANSFER_WORK {
                return None;
            }
        }
    }
    if work > MAX_PARTITION_TRANSFER_WORK {
        return None;
    }
    *retained = work;
    Some(())
}

#[test]
fn symbolic_return_capture_charges_retained_rows_before_committing() {
    let row = vec![Rc::new(IntegerSet::singleton(&BigInt::from(1))); 16];
    let small = IntegerInputs::from_cells(vec![row.clone(); 16]);
    let large = IntegerInputs::from_cells(vec![row.clone(); 128]);
    let mut retained = 0;
    assert_eq!(charge_partition_capture(&mut retained, &small, row.len()), Some(()));
    let first = retained;
    assert_eq!(charge_partition_capture(&mut retained, &small, row.len()), Some(()));
    assert_eq!(retained, first * 2);
    assert_eq!(charge_partition_capture(&mut retained, &large, row.len()), None);
    assert_eq!(retained, first * 2);
}

enum ScalarOutputRows {
    Bound(IntegerInputs),
    #[cfg(feature = "internal-checker")]
    Guard { lanes: [IntegerInputs; 2], work: usize },
}

#[cfg(feature = "internal-checker")]
fn bool_result_lane(value: &BigInt) -> Option<usize> {
    use num_bigint::Sign::{NoSign, Plus};
    match (value.sign(), value.bits()) {
        (NoSign, 0) => Some(0),
        (Plus, 1) => Some(1),
        _ => None,
    }
}

impl ScalarOutputRows {
    fn push_singleton(&mut self, mut row: IntegerRow, output: Rc<IntegerSet>) -> Option<()> {
        match self {
            Self::Bound(rows) => { row.push(output); rows.0.push(row); }
            #[cfg(feature = "internal-checker")]
            Self::Guard { lanes, work } => {
                charge_call_work(work, 1)?;
                let lane = bool_result_lane(output.singleton_value()?)?;
                lanes[lane].0.push(row);
            }
        }
        Some(())
    }

    fn push_alias(&mut self, mut row: IntegerRow, axis: usize) -> Option<()> {
        match self {
            Self::Bound(rows) => {
                row.append_alias(axis, Rc::clone(&row[axis]));
                rows.0.push(row);
            }
            #[cfg(feature = "internal-checker")]
            Self::Guard { lanes, work } => {
                charge_call_work(work, 1)?;
                if let Some(exact) = row[axis].singleton_value() {
                    lanes[bool_result_lane(exact)?].0.push(row);
                    return Some(());
                }
                let (low, high) = row[axis].extremes()?;
                if (bool_result_lane(low)?, bool_result_lane(high)?) != (0, 1) || row.affine_touches(axis) {
                    return None;
                }
                // Only a truly broad Boolean alias needs two rows. Restrict
                // its existing typed axis so aliases of that input refine.
                let cost = row.len().checked_add(2usize.checked_mul(row.equal_axes().len())?)?.checked_add(4)?;
                charge_call_work(work, cost)?;
                let mut no = row.clone();
                no.replace_domain(axis, Rc::new(IntegerSet::singleton(&0u8.into())));
                row.replace_domain(axis, Rc::new(IntegerSet::singleton(&1u8.into())));
                lanes[0].0.push(no);
                lanes[1].0.push(row);
            }
        }
        Some(())
    }

    fn push_affine(&mut self, mut row: IntegerRow, plan: AffineAppend) -> Option<()> {
        match self {
            Self::Bound(rows) => { row.append_affine(plan); rows.0.push(row); Some(()) }
            #[cfg(feature = "internal-checker")]
            Self::Guard { .. } => None,
        }
    }
}

pub(super) struct ExactCallCursor<'a> {
    roots: IntegerRoots<'a>,
    block: IntegerBlockCursor,
    tail: Option<&'a ScalarType>,
    discarded_return: Option<bool>,
    returned: Vec<IntegerReturningPath<'a>>,
    computed_returns: Vec<(usize, ComputedIntegerReturn)>,
    return_capture_work: usize,
    projected_slot: Option<usize>,
    projected_pair: Option<[usize; 2]>,
}

impl<'a> ExactCallCursor<'a> {
    pub(super) fn new(calls: &'a FunctionCallSites, exact: &[&BigInt], expected: &ScalarType) -> Option<Self> {
        Self::with_row(calls, expected, exact_integer_entry_row(exact), None)
    }

    pub(super) fn arbitrary_root(calls: &'a FunctionCallSites, expected: &ScalarType) -> Option<Self> {
        if !matches!(*expected, ScalarType::Unit | ScalarType::Bool) { return None; }
        let row = calls.boundary.parameters.iter()
            .map(|parameter| ordinary_parameter_set(&parameter.ty, true).map(Rc::new))
            .collect::<Option<IntegerRow>>()?;
        let mut cursor = Self::with_row(calls, expected, row, None)?;
        if *expected == ScalarType::Bool { cursor.discarded_return = Some(false); }
        Some(cursor)
    }

    pub(super) fn projected(calls: &'a FunctionCallSites, values: Rc<IntegerSet>, expected: &ScalarType) -> Option<Self> {
        Self::with_row(calls, expected, IntegerRow::new(vec![values]), Some(0))
    }

    pub(super) fn projected_arguments(
        calls: &'a FunctionCallSites, values: Rc<IntegerSet>, argument: &ProjectedIntegerArgument<'_>, expected: &ScalarType,
    ) -> Option<Self> {
        let row = match (argument.slot, argument.companion) {
            (0, ProjectedCompanion::Unary) => IntegerRow::new(vec![values]),
            (0, ProjectedCompanion::Fixed((_, value))) => IntegerRow::new(vec![values, Rc::new(IntegerSet::singleton(value))]),
            (1, ProjectedCompanion::Fixed((_, value))) => IntegerRow::new(vec![Rc::new(IntegerSet::singleton(value)), values]),
            (0, ProjectedCompanion::Equal) => {
                let mut row = IntegerRow::new(vec![Rc::clone(&values)]);
                row.append_alias(0, values);
                row
            }
            _ => return None,
        };
        Self::with_row(calls, expected, row, Some(argument.slot))
    }

    pub(super) fn projected_pair(
        calls: &'a FunctionCallSites, argument: &mut ProjectedIntegerArgument<'_>, other: usize, expected: &ScalarType,
    ) -> Option<Self> {
        let axes = [argument.caller_axis, other];
        if argument.path.0.len().checked_mul(5)? > MAX_PARTITION_TRANSFER_WORK {
            return None;
        }
        let mut work: usize = 0;
        let mut affine_work = argument.affine_work.as_deref_mut();
        let mut seeded_affine = false;
        let rows = argument.path.0.iter().enumerate().map(|(index, row)| {
            let cost = 3usize.checked_add(row[axes[0]].interval_count())?
                .checked_add(row[axes[1]].interval_count())?;
            if let Some(affine_work) = affine_work.as_deref_mut() {
                let comparisons = usize::BITS - row.equal_axes().len().leading_zeros();
                charge_call_work(affine_work, cost.checked_add(2 * comparisons as usize)?)?;
            } else {
                work = work.checked_add(cost)?;
                if work > MAX_PARTITION_TRANSFER_WORK { return None; }
            }
            let first_axis = row.canonical_axis(axes[0]);
            let second_axis = row.canonical_axis(axes[1]);
            let same = first_axis == second_axis;
            let projected_class = if let Some(class) = row.affine.as_deref() {
                if !same && class.contains(first_axis) && class.contains(second_axis) {
                    charge_call_work(affine_work.as_deref_mut()?, 1)?;
                    let transform = class.between(first_axis, second_axis)?;
                    seeded_affine = true;
                    Some(Rc::new(IntegerAffineAxes { anchor: 0, members: vec![(1, transform)] }))
                } else {
                    None
                }
            } else {
                None
            };
            let first = Rc::clone(&row[axes[0]]);
            let second = Rc::clone(&row[axes[1]]);
            let mut next = if same {
                let mut next = IntegerRow::new(vec![first]);
                next.append_alias(0, second);
                next
            } else {
                IntegerRow::new(vec![first, second])
            };
            next.affine = projected_class;
            next.projected_from = u16::try_from(index.checked_add(1)?).ok()?;
            Some(next)
        }).collect::<Option<Vec<_>>>()?;
        let mut child = Self::with_rows(calls, expected, IntegerInputs(rows), None, Some(axes))?;
        if seeded_affine { child.roots.affine_work = Some(0); }
        Some(child)
    }

    fn with_row(calls: &'a FunctionCallSites, expected: &ScalarType, row: IntegerRow, projected_slot: Option<usize>) -> Option<Self> {
        Self::with_rows(calls, expected, IntegerInputs(vec![row]), projected_slot, None)
    }

    fn with_rows(
        calls: &'a FunctionCallSites, expected: &ScalarType, rows: IntegerInputs,
        projected_slot: Option<usize>, projected_pair: Option<[usize; 2]>,
    ) -> Option<Self> {
        let tail = if *expected == ScalarType::Unit { None } else {
            Some(ordinary_tail_type(calls, true).filter(|tail| *tail == expected)?)
        };
        let boundary = &calls.boundary;
        let bindings = boundary.checker_parameter_bindings.as_deref()?;
        let parameters = boundary.parameters.len();
        if bindings.len() != parameters || rows.0.iter().any(|row| row.len() != parameters) {
            return None;
        }
        let mut roots = IntegerRoots {
            parameters, projected_entry_axis: projected_slot, unit_pair_work: projected_pair.map(|_| 0),
            ..IntegerRoots::default()
        };
        for (binding, parameter) in bindings.iter().zip(&boundary.parameters) {
            roots.add(binding, &parameter.ty);
        }
        let block = IntegerBlockCursor::new(calls.structure.root, roots.axes.len(), rows);
        Some(Self { roots, block, tail, discarded_return: None, returned: Vec::new(), computed_returns: Vec::new(), return_capture_work: 0, projected_slot, projected_pair })
    }

    pub(super) fn arguments<'borrow>(
        &'borrow mut self, calls: &'borrow FunctionCallSites, call: OccurrenceOrdinal, parameters: usize,
        collect: bool, project: bool, project_pair: bool, project_independent: bool, allow_bool: bool,
    ) -> Option<NestedIntegerArgument<'borrow>> {
        let mut work = self.roots.affine_work;
        let mut argument = if collect && parameters == 2 {
            nested_pair_argument(calls, call, &self.roots, &self.block.path,
                project_pair, project_pair, project_independent, allow_bool, work.as_mut())?
        } else {
            actual_nested_argument(calls, call, &self.roots, &self.block.path, project, allow_bool, work.as_mut())
        };
        self.roots.affine_work = work;
        if let Some(work) = &mut self.roots.affine_work {
            if *work > MAX_PARTITION_TRANSFER_WORK { return None; }
            match &mut argument {
                NestedIntegerArgument::Projected(projection) if project_pair =>
                    projection.affine_work = Some(work),
                NestedIntegerArgument::Single(Some(_)) | NestedIntegerArgument::Pair(..) => {}
                NestedIntegerArgument::Single(None) if parameters == 0 => {}
                _ if self.block.path.0.iter().any(|row| row.affine.is_some()) => return None,
                _ => {}
            }
        }
        Some(argument)
    }

    fn scalar_output_rows(&self, binding: Option<&'a ReferenceBindingId>, ty: &ScalarType) -> Option<ScalarOutputRows> {
        if binding.is_some() { return Some(ScalarOutputRows::Bound(IntegerInputs::default())); }
        #[cfg(feature = "internal-checker")]
        {
            let guard = self.block.guard.as_ref()?;
            if *ty != ScalarType::Bool || guard.next.is_some() || guard.returned.is_some() { return None; }
            Some(ScalarOutputRows::Guard {
                lanes: [IntegerInputs::default(), IntegerInputs::default()],
                work: self.roots.ordinary_comparison_work?,
            })
        }
        #[cfg(not(feature = "internal-checker"))]
        { let _ = ty; None }
    }

    fn finish_scalar_rows(
        &mut self, joined: ScalarOutputRows, binding: Option<&'a ReferenceBindingId>, ty: &'a ScalarType,
    ) -> Option<()> {
        match joined {
            ScalarOutputRows::Bound(rows) => {
                self.roots.add(binding?, ty);
                self.block.path = rows;
            }
            #[cfg(feature = "internal-checker")]
            ScalarOutputRows::Guard { lanes, work } => {
                self.block.path = IntegerInputs::default();
                self.block.guard.as_mut()?.returned = Some(lanes);
                self.roots.ordinary_comparison_work = Some(work);
            }
        }
        Some(())
    }

    pub(super) fn resume(&mut self, effect: ExactCallEffect<'a>, caller_axis: Option<usize>, result: ActualCallResult) -> Option<()> {
        match (effect, result) {
            (ExactCallEffect::Unit(_), ActualCallResult::Unit) => {}
            (ExactCallEffect::Unit(_), ActualCallResult::EmptyUnit) => self.block.path = IntegerInputs::default(),
            (ExactCallEffect::Scalar { binding, ty, .. }, ActualCallResult::Integer(value)) => {
                if let Some(binding) = binding {
                    self.roots.add(binding, ty);
                    self.block.path.append_local(IntegerSet::singleton(&value));
                } else {
                    #[cfg(feature = "internal-checker")]
                    {
                        let ScalarOutputRows::Guard { mut lanes, mut work } = self.scalar_output_rows(binding, ty)?
                            else { return None; };
                        let lane = bool_result_lane(&value)?;
                        charge_call_work(&mut work, 1)?;
                        lanes[lane] = std::mem::take(&mut self.block.path);
                        self.finish_scalar_rows(ScalarOutputRows::Guard { lanes, work }, binding, ty)?;
                    }
                    #[cfg(not(feature = "internal-checker"))]
                    return None;
                }
            }
            (ExactCallEffect::Unit(_), ActualCallResult::ProjectedUnit(survivors)) => {
                let axis = caller_axis?;
                if let Some(work) = &mut self.roots.affine_work {
                    let survivor_intervals = survivors.interval_count();
                    for row in &self.block.path.0 {
                        let comparisons = row[axis].interval_count().checked_add(survivor_intervals)?;
                        *work = work.checked_add(comparisons)?.checked_add(row.equal_axes().len())?.checked_add(1)?;
                        row.charge_affine_replacement(axis, comparisons, work)?;
                        if *work > MAX_PARTITION_TRANSFER_WORK {
                            return None;
                        }
                    }
                }
                #[cfg(feature = "internal-checker")]
                self.block.path.charge_local_alias_work(&mut self.roots.ordinary_local_work, 1)?;
                self.block.path.restrict_axis(axis, &survivors);
            }
            (ExactCallEffect::Unit(_), ActualCallResult::ProjectedUnitPair(projected)) => {
                let original = &self.block.path.0;
                let mut work = self.roots.affine_work.unwrap_or(0);
                for survivor in &projected.survivors.0 {
                    let index = usize::from(survivor.projected_from).checked_sub(1)?;
                    let row = original.get(index)?;
                    work = work.checked_add(row.len())?.checked_add(2usize.checked_mul(row.equal_axes().len())?)?
                        .checked_add(survivor[0].interval_count())?.checked_add(survivor[1].interval_count())?;
                    row.charge_affine_pair(projected.caller_axes,
                        [survivor[0].interval_count(), survivor[1].interval_count()], &mut work)?;
                    if work > MAX_PARTITION_TRANSFER_WORK { return None; }
                }
                if let Some(retained) = &mut self.roots.affine_work { *retained = work; }
                self.block.path.0 = projected.survivors.0.iter().map(|survivor| {
                    let mut row = original[usize::from(survivor.projected_from) - 1].clone();
                    row.replace_pair(projected.caller_axes, [Rc::clone(&survivor[0]), Rc::clone(&survivor[1])])?;
                    Some(row)
                }).collect::<Option<Vec<_>>>()?;
            }
            (ExactCallEffect::Scalar { binding, ty, .. }, ActualCallResult::PartitionedIntegerPair(projected)) => {
                let original = &self.block.path.0;
                let mut work = self.roots.affine_work.unwrap_or(0);
                let mut uses_affine = self.roots.affine_work.is_some();
                let mut joined = self.scalar_output_rows(binding, ty)?;
                for piece in projected.pieces {
                    let index = usize::from(piece.anchor).checked_sub(1)?;
                    let row = original.get(index)?;
                    let intervals = [piece.inputs[0].interval_count(), piece.inputs[1].interval_count()];
                    let base = row.len().checked_add(1)?
                        .checked_add(3usize.checked_mul(row.equal_axes().len())?)?
                        .checked_add(intervals[0])?.checked_add(intervals[1])?;
                    charge_call_work(&mut work, base)?;
                    row.charge_affine_pair(projected.caller_axes, intervals, &mut work)?;
                    if work > MAX_PARTITION_TRANSFER_WORK { return None; }
                    let plan = match &piece.output {
                        IntegerPairPartitionOutput::InputAlias(slot) | IntegerPairPartitionOutput::InputAffine(slot, _) => {
                            if self.roots.axes.get(projected.caller_axes[*slot])?.1 != ty { return None; }
                            charge_call_work(&mut work, 1)?;
                            if let IntegerPairPartitionOutput::InputAffine(_, transform) = &piece.output {
                                let (min, max) = super::typecheck::integer_bounds(ty)?;
                                transform.fits(&piece.inputs[*slot], &min, &max)?;
                                uses_affine = true;
                                Some(row.prepare_affine(
                                    projected.caller_axes[*slot], &piece.inputs[*slot], transform, &mut work,
                                )?)
                            } else {
                                None
                            }
                        }
                        IntegerPairPartitionOutput::Singleton(_) => None,
                    };
                    let mut row = row.clone();
                    row.replace_pair(projected.caller_axes, piece.inputs)?;
                    match piece.output {
                        IntegerPairPartitionOutput::Singleton(output) => joined.push_singleton(row, output)?,
                        IntegerPairPartitionOutput::InputAlias(slot) => joined.push_alias(row, projected.caller_axes[slot])?,
                        IntegerPairPartitionOutput::InputAffine(..) => joined.push_affine(row, plan?)?,
                    }
                }
                if uses_affine { self.roots.affine_work = Some(work); }
                self.finish_scalar_rows(joined, binding, ty)?;
            }
            (ExactCallEffect::Scalar { binding, ty, .. }, ActualCallResult::PartitionedInteger(pieces)) => {
                let axis = caller_axis?;
                let (piece_intervals, depends_on_input, affine) = pieces.iter().try_fold(
                    (0usize, false, false), |(sum, depends, affine), piece| Some((
                        sum.checked_add(piece.input.interval_count())?,
                        depends || !matches!(&piece.output, IntegerPartitionOutput::Singleton(_)),
                        affine || matches!(&piece.output, IntegerPartitionOutput::InputAffine(_)),
                    )))?;
                if depends_on_input && self.roots.axes.get(axis)?.1 != ty {
                    return None;
                }
                let rows = &self.block.path.0;
                let cells = self.roots.axes.len().checked_add(1)?;
                let row_intervals = rows.iter().try_fold(0usize, |sum, row| sum.checked_add(row[axis].interval_count()))?;
                let comparisons = row_intervals.checked_mul(pieces.len())?
                    .checked_add(piece_intervals.checked_mul(rows.len())?)?;
                let bounds = if affine { Some(super::typecheck::integer_bounds(ty)?) } else { None };
                let mut retained = self.roots.affine_work;
                if affine || retained.is_some() {
                    charge_call_work(retained.get_or_insert(0), comparisons.checked_add(rows.len().checked_mul(pieces.len())?)?)?;
                } else {
                    let row_copies = rows.len().checked_mul(pieces.len())?.checked_mul(cells)?;
                    let alias_work = rows.iter().try_fold(0usize, |sum, row| sum.checked_add(row.equal_axes().len()))?
                        .checked_mul(pieces.len())?;
                    if row_copies.checked_add(comparisons)?.checked_add(alias_work)? > MAX_PARTITION_TRANSFER_WORK {
                        return None;
                    }
                }
                let original = std::mem::take(&mut self.block.path.0);
                let mut joined = self.scalar_output_rows(binding, ty)?;
                for row in &original {
                    for piece in &pieces {
                        let narrowed = match row[axis].intersection(&piece.input) {
                            IntegerIntersection::Unchanged => None,
                            IntegerIntersection::Empty => continue,
                            IntegerIntersection::Changed(values) => Some(values),
                        };
                        let mut plan = None;
                        if let Some(work) = &mut retained {
                            charge_call_work(work, cells)?;
                            if let Some(values) = &narrowed {
                                charge_call_work(work, row.equal_axes().len())?;
                                row.charge_affine_replacement(axis, values.interval_count(), work)?;
                            }
                            if matches!(&piece.output, IntegerPartitionOutput::InputAlias) {
                                charge_call_work(work, row.equal_axes().len())?;
                            }
                            if let IntegerPartitionOutput::InputAffine(transform) = &piece.output {
                                let (min, max) = bounds.as_ref()?;
                                let values = narrowed.as_ref().unwrap_or_else(|| row[axis].as_ref());
                                transform.fits(values, min, max)?;
                                plan = Some(row.prepare_affine(axis, values, transform, work)?);
                            }
                            if *work > MAX_PARTITION_TRANSFER_WORK { return None; }
                        }
                        let mut next = row.clone();
                        if let Some(values) = narrowed {
                            next.replace_domain(axis, Rc::new(values));
                        }
                        match &piece.output {
                            IntegerPartitionOutput::Singleton(output) => joined.push_singleton(next, Rc::clone(output))?,
                            IntegerPartitionOutput::InputAlias => joined.push_alias(next, axis)?,
                            IntegerPartitionOutput::InputAffine(_) => joined.push_affine(next, plan?)?,
                        }
                    }
                }
                self.roots.affine_work = retained;
                self.finish_scalar_rows(joined, binding, ty)?;
            }
            _ => return None,
        }
        Some(())
    }

    pub(super) fn advance<const COLLECT: bool>(
        &mut self,
        calls: &'a FunctionCallSites,
        mut handle_call: Option<&mut ActualCallHandler<'a, '_>>,
        allow_contextual_local_transfers: bool,
        suspend_integer: bool,
        allow_ordinary_computed_returns: bool,
        allow_ordinary_integer_locals: bool,
        sink: &mut dyn FnMut(IntegerCheckFailure<'a>),
    ) -> Option<ExactCallProgress<'a>> {
        #[cfg(feature = "internal-checker")]
        {
            self.roots.allow_ordinary_integer_locals = allow_ordinary_integer_locals && !calls.structure.checker_ordinary_scalar_locals.is_empty();
            self.roots.allow_ordinary_affine_overwrites = allow_ordinary_integer_locals;
            if allow_ordinary_integer_locals && calls.structure.has_ordinary_predicates() {
                self.roots.ordinary_comparison_work.get_or_insert(0);
            }
        }
        #[cfg(not(feature = "internal-checker"))]
        let _ = allow_ordinary_integer_locals;
        if self.tail == Some(&ScalarType::Bool) && !allow_ordinary_integer_locals { return None; }
        let mut failures = IntegerFailureCollection {
            continuation: IntegerRequirementContinuation::Empty,
            projected: None, allow_projected_calls: false, allow_contextual_local_transfers, sink,
        };
        let mut imports: &mut [Option<ImportedLocal<'a>>] = &mut [];
        let returns = (self.tail.is_some() && self.discarded_return.is_none()).then_some(&mut self.returned);
        let control = drive_integer_block::<false, COLLECT, _, true>(
            calls, &mut self.roots, &mut self.block, self.tail, returns, &mut handle_call,
            &mut Some(&mut failures), &mut imports, (), suspend_integer,
            (self.projected_slot.is_some() || self.projected_pair.is_some()).then_some(&mut self.return_capture_work),
            (allow_ordinary_computed_returns && suspend_integer && self.tail.is_some_and(is_integer_type)
                && (self.projected_slot.is_some() || self.projected_pair.is_some()))
                .then_some(&mut self.computed_returns),
            self.discarded_return.as_mut(),
        )?;
        let result = match (self.tail, control) {
            (_, IntegerControl::PendingCall(effect)) => return Some(ExactCallProgress::Pending(effect)),
            (_, IntegerControl::Fail(failure)) => return Some(ExactCallProgress::Failed(failure)),
            (None, IntegerControl::Continue(path)) if COLLECT && path.is_unreachable() => ActualCallResult::EmptyUnit,
            (None, IntegerControl::Continue(survivors)) if self.projected_pair.is_some() =>
                ActualCallResult::ProjectedUnitPair(ProjectedUnitPair { caller_axes: self.projected_pair?, survivors }),
            (None, IntegerControl::Continue(path)) if self.projected_slot.is_some() =>
                ActualCallResult::ProjectedUnit(path.project_entry(self.projected_slot?)?),
            (None, IntegerControl::Continue(_) | IntegerControl::Returned) => ActualCallResult::Unit,
            (Some(_), IntegerControl::Returned) if self.discarded_return == Some(true) =>
                ActualCallResult::Unit,
            (Some(_), IntegerControl::Continue(path)) if self.discarded_return.is_some() && path.is_unreachable() =>
                ActualCallResult::EmptyUnit,
            (Some(_), IntegerControl::Returned) if self.projected_pair.is_some() => {
                let caller_axes = self.projected_pair?;
                let piece_count = self.returned.iter().try_fold(self.computed_returns.len(), |count, path| count.checked_add(path.state.0.len()))?;
                if piece_count == 0 || piece_count > MAX_PARTITION_TRANSFER_WORK {
                    return None;
                }
                let mut pieces = Vec::with_capacity(piece_count);
                let mut computed = self.computed_returns.drain(..).peekable();
                for (index, path) in self.returned.iter().enumerate() {
                    while computed.peek().is_some_and(|(at, _)| *at == index) {
                        let ComputedIntegerReturn::Pair(piece) = computed.next()?.1 else { return None; };
                        pieces.push(piece);
                    }
                    let (alias, constant) = match path.value {
                        IntegerReturn::Const(value) => (None, Some(Rc::new(IntegerSet::singleton(value)))),
                        IntegerReturn::Alias(binding) => (Some(path.axes.iter().position(|axis| *axis == binding)?), None),
                    };
                    for row in &path.state.0 {
                        let output = if let Some(axis) = alias {
                            if row[axis].singleton_value().is_some() {
                                IntegerPairPartitionOutput::Singleton(Rc::clone(&row[axis]))
                            } else if let Some(slot) = (0..2).find(|slot| row.entry_alias(*slot, axis)) {
                                IntegerPairPartitionOutput::InputAlias(slot)
                            } else if allow_ordinary_computed_returns {
                                let (slot, transform) = (0..2).find_map(|slot| {
                                    row.entry_affine(slot, axis).map(|transform| (slot, transform))
                                })?;
                                IntegerPairPartitionOutput::InputAffine(slot, transform)
                            } else {
                                return None;
                            }
                        } else {
                            IntegerPairPartitionOutput::Singleton(Rc::clone(constant.as_ref()?))
                        };
                        pieces.push(IntegerPairReturnPartition {
                            anchor: row.projected_from,
                            inputs: [Rc::clone(&row[0]), Rc::clone(&row[1])],
                            output,
                        });
                    }
                }
                for (_, piece) in computed {
                    let ComputedIntegerReturn::Pair(piece) = piece else { return None; };
                    pieces.push(piece);
                }
                ActualCallResult::PartitionedIntegerPair(ProjectedIntegerPair { caller_axes, pieces })
            }
            (Some(_), IntegerControl::Returned) if self.projected_slot.is_some() => {
                let slot = self.projected_slot?;
                let piece_count = self.returned.iter().try_fold(self.computed_returns.len(), |count, path| count.checked_add(path.state.0.len()))?;
                if piece_count == 0 || piece_count > MAX_PARTITION_TRANSFER_WORK {
                    return None;
                }
                let mut pieces = Vec::with_capacity(piece_count);
                let mut computed = self.computed_returns.drain(..).peekable();
                for (index, path) in self.returned.iter().enumerate() {
                    while computed.peek().is_some_and(|(at, _)| *at == index) {
                        let ComputedIntegerReturn::Projected(piece) = computed.next()?.1 else { return None; };
                        pieces.push(piece);
                    }
                    let alias = match path.value {
                        IntegerReturn::Const(_) => None,
                        IntegerReturn::Alias(binding) => Some(path.axes.iter().position(|axis| *axis == binding)?),
                    };
                    let constant = match path.value {
                        IntegerReturn::Const(value) => Some(Rc::new(IntegerSet::singleton(value))),
                        IntegerReturn::Alias(_) => None,
                    };
                    for row in &path.state.0 {
                        let input = Rc::clone(row.entry_domain(slot));
                        let output = if let Some(axis) = alias {
                            if row.entry_alias(slot, axis) && input.singleton_value().is_none() {
                                IntegerPartitionOutput::InputAlias
                            } else if row[axis].singleton_value().is_some() {
                                IntegerPartitionOutput::Singleton(Rc::clone(&row[axis]))
                            } else if allow_ordinary_computed_returns {
                                IntegerPartitionOutput::InputAffine(row.entry_affine(slot, axis)?)
                            } else {
                                return None;
                            }
                        } else {
                            IntegerPartitionOutput::Singleton(Rc::clone(constant.as_ref()?))
                        };
                        pieces.push(IntegerReturnPartition { span: path.span, input, output });
                    }
                }
                for (_, piece) in computed {
                    let ComputedIntegerReturn::Projected(piece) = piece else { return None; };
                    pieces.push(piece);
                }
                ActualCallResult::PartitionedInteger(pieces)
            }
            (Some(_), IntegerControl::Returned) => {
                let [path] = self.returned.as_slice() else { return None; };
                ActualCallResult::Integer(path.singleton_return()?.clone())
            }
            (Some(_), IntegerControl::Continue(_)) => return None,
        };
        Some(ExactCallProgress::Done { active: self.roots.reached_obligation, result })
    }
}

#[cfg(feature = "internal-checker")]
type IntegerCalculationOperand<'a> = (Option<&'a BigInt>, Option<usize>);

#[cfg(feature = "internal-checker")]
fn ordinary_local_operand<'a>(
    calls: &FunctionCallSites, roots: &IntegerRoots<'_>, expected: &ScalarType,
    operand: &'a super::reference::CheckerOrdinaryIntegerOperand,
) -> Option<IntegerCalculationOperand<'a>> {
    use super::reference::CheckerOrdinaryIntegerOperand::{Lexical, Literal};
    match operand {
        Literal(value) => Some((Some(value), None)),
        Lexical(occurrence) => {
            let PlainNameIdentity::Lexical(binding) = calls.prepared_plain_name(*occurrence)? else { return None; };
            let (axis, ty) = roots.lookup(binding)?;
            (ty == expected && calls.structure.checked_types.expressions.get(occurrence)? == expected)
                .then_some((None, Some(axis)))
        }
    }
}

#[cfg(feature = "internal-checker")]
fn singleton_calculation_operand<'a>(
    row: &'a IntegerRow, (literal, axis): IntegerCalculationOperand<'a>,
) -> Option<&'a BigInt> {
    literal.or_else(|| row.get(axis?)?.singleton_value())
}

#[cfg(feature = "internal-checker")]
enum ComputedProjectedValue {
    Singleton(Rc<IntegerSet>),
    Identity(usize),
    Affine(usize, Rc<IntegerAffineTransform>),
}

#[cfg(feature = "internal-checker")]
enum ComputedRowInteger {
    Singleton(BigInt),
    Affine(usize, IntegerAffineTransform),
}

#[cfg(feature = "internal-checker")]
fn compute_row_integer_operation(
    row: &IntegerRow, operation: AuthoredIntegerOperation,
    left: IntegerCalculationOperand<'_>, right: IntegerCalculationOperand<'_>,
    min: &BigInt, max: &BigInt,
) -> Option<ComputedRowInteger> {
    let fits = |value: &BigInt| value >= min && value <= max;
    let left_exact = singleton_calculation_operand(row, left);
    let right_exact = singleton_calculation_operand(row, right);
    let (axis, reflected, offset) = match (left_exact, right_exact, operation) {
        (Some(left), Some(right), operation) if fits(left) && fits(right) => {
            let value = operation.apply(Cow::Borrowed(left), Cow::Borrowed(right), min)?;
            return fits(&value).then(|| ComputedRowInteger::Singleton(value));
        }
        (None, Some(value), AuthoredIntegerOperation::Add) if fits(value) => (left.1?, false, value.clone()),
        (Some(value), None, AuthoredIntegerOperation::Add) if fits(value) => (right.1?, false, value.clone()),
        (None, Some(value), AuthoredIntegerOperation::Subtract) if fits(value) => (left.1?, false, -value),
        (Some(value), None, AuthoredIntegerOperation::Subtract) if fits(value) => (right.1?, true, value.clone()),
        _ => return None,
    };
    let transform = IntegerAffineTransform { reflected, offset };
    transform.fits(&row[axis], min, max)?;
    Some(ComputedRowInteger::Affine(axis, transform))
}

#[cfg(feature = "internal-checker")]
impl IntegerInputs {
    fn append_ordinary_calculation(
        &mut self, operation: AuthoredIntegerOperation,
        left: IntegerCalculationOperand<'_>, right: IntegerCalculationOperand<'_>, ty: &ScalarType,
        local_work: &mut Option<usize>, affine_work: &mut Option<usize>,
    ) -> Option<()> {
        let work = local_work.get_or_insert(0);
        // At most two operand inspections, one typed calculation, one output
        // cell and one pass through the input rows. Integer types end at 128 bits.
        charge_call_work(work, self.0.len().checked_mul(5)?.checked_add(1)?)?;
        let (min, max) = super::typecheck::integer_bounds(ty)?;
        for row in &mut self.0 {
            match compute_row_integer_operation(row, operation, left, right, &min, &max)? {
                ComputedRowInteger::Singleton(value) => row.push(Rc::new(IntegerSet::singleton(&value))),
                ComputedRowInteger::Affine(axis, transform)
                    if !transform.reflected && transform.offset.bits() == 0 =>
                {
                    // An identity of a non-anchor member is still an equality
                    // copy, not another member with a redundant transform.
                    let meter = affine_work.as_mut().filter(|_| row.affine.is_some()).unwrap_or(&mut *work);
                    charge_call_work(meter, row.equal_axes().len().checked_add(1)?)?;
                    row.append_alias(axis, Rc::clone(&row[axis]));
                }
                ComputedRowInteger::Affine(axis, transform) => {
                    let transform = Rc::new(transform);
                    let meter = affine_work.as_mut().filter(|_| row.affine.is_some()).unwrap_or(&mut *work);
                    let plan = row.prepare_affine(axis, &row[axis], &transform, meter)?;
                    if matches!(&plan, AffineAppend::Member { .. }) {
                        affine_work.get_or_insert(0);
                    }
                    row.append_affine(plan);
                }
            }
        }
        // A declined row abandons the cursor; no partially evaluated lane or
        // collected diagnostics are published on an Unsupported result.
        Some(())
    }
}

#[cfg(feature = "internal-checker")]
fn computed_projected_value(
    row: &IntegerRow, formal: Option<usize>, operation: AuthoredIntegerOperation,
    left: IntegerCalculationOperand<'_>, right: IntegerCalculationOperand<'_>,
    min: &BigInt, max: &BigInt,
) -> Option<ComputedProjectedValue> {
    let (axis, mut transform) = match compute_row_integer_operation(row, operation, left, right, min, max)? {
        ComputedRowInteger::Singleton(value) =>
            return Some(ComputedProjectedValue::Singleton(Rc::new(IntegerSet::singleton(&value)))),
        ComputedRowInteger::Affine(axis, transform) => (axis, transform),
    };
    let entry_relation = |slot| {
        if row.entry_alias(slot, axis) {
            Some((slot, None))
        } else {
            Some((slot, Some(row.entry_affine(slot, axis)?)))
        }
    };
    let (origin, inner) = match formal {
        Some(formal) => entry_relation(formal)?,
        None => (0..2).find(|slot| row.entry_alias(*slot, axis)).map(|slot| (slot, None))
            .or_else(|| (0..2).find_map(entry_relation))?,
    };
    if let Some(inner) = inner {
        transform = transform.compose(&inner);
    }
    if !transform.reflected && transform.offset.bits() == 0 {
        Some(ComputedProjectedValue::Identity(origin))
    } else {
        Some(ComputedProjectedValue::Affine(origin, Rc::new(transform)))
    }
}

fn exact_integer_entry_row(exact: &[&BigInt]) -> IntegerRow {
    exact.iter().map(|value| Rc::new(IntegerSet::singleton(value))).collect()
}

fn integer_assignment(
    calls: &FunctionCallSites,
    roots: &IntegerRoots<'_>,
    position: BlockPosition,
    path: &IntegerInputs,
    allow_contextual_local_transfers: impl FnOnce() -> bool,
) -> Option<(usize, Rc<IntegerSet>, OccurrenceOrdinal, OccurrenceOrdinal)> {
    let (binding, replacement, rhs, destination) = checked_scalar_assignment(calls, position)?;
    let (axis, ty) = roots.lookup(binding)?;
    let value = if let Some(value) = calls.structure.checker_integer_literals.get(&rhs) {
        Rc::new(IntegerSet::singleton(value))
    } else {
        if !calls.structure.checker_contextual_occurrences.contains(&destination)
            || (!calls.structure.checker_contextual_occurrences.contains(&rhs) && !allow_contextual_local_transfers())
        {
            return None;
        }
        let PlainNameIdentity::Lexical(source) = calls.prepared_plain_name(rhs)? else { return None; };
        let (source, source_ty) = roots.lookup(source)?;
        if source_ty != replacement {
            return None;
        }
        Rc::clone(path.singleton_axis(source)?.1)
    };
    (ty == replacement).then_some((axis, value, rhs, destination))
}

#[cfg(feature = "internal-checker")]
fn checked_literal_assignment(
    calls: &FunctionCallSites, position: BlockPosition,
) -> Option<(&ReferenceBindingId, &ScalarType, &BigInt)> {
    let (binding, ty, rhs, _) = checked_scalar_assignment(calls, position)?;
    Some((binding, ty, calls.structure.checker_integer_literals.get(&rhs)?))
}

fn checked_scalar_assignment(
    calls: &FunctionCallSites, position: BlockPosition,
) -> Option<(&ReferenceBindingId, &ScalarType, OccurrenceOrdinal, OccurrenceOrdinal)> {
    let group = calls.assignments.get(&(position.block, position.item))?;
    let [rhs] = group.rhs_evaluations().as_slice() else { return None; };
    let [destination] = group.destinations().as_slice() else { return None; };
    if group.projected_outputs().len() != 0 {
        return None;
    }
    let DirectDestinationIdentity::Existing(binding) =
        calls.prepared_direct_destination(destination.destination)?
    else { return None; };
    let AssignmentReplacement::Resolved(replacement) = &destination.replacement else { return None; };
    Some((binding, replacement, rhs.expression, destination.destination))
}

#[cfg(feature = "internal-checker")]
pub(super) fn backward_literal_overwrite_failure(
    calls: &FunctionCallSites, failure: &IntegerCheckFailure<'_>,
) -> Option<(ScalarType, BigInt, BigInt, BigInt)> {
    let structure = &calls.structure;
    let [narrow, write, require] = structure.blocks.get(structure.root.0)?.items.as_slice() else { return None; };
    let BlockItemKind::Evaluation { expression: narrow_expression } = narrow.kind else { return None; };
    let BlockItemKind::Assignment { group } = write.kind else { return None; };
    let BlockItemKind::Evaluation { expression: require_expression } = require.kind else { return None; };
    if require_expression != failure.directive {
        return None;
    }
    let narrow = structure.checker_directives.get(&narrow_expression)?;
    let require = structure.checker_directives.get(&require_expression)?;
    if narrow.operation != CheckerOperation::Narrow || require.operation != CheckerOperation::Require {
        return None;
    }
    let CheckerPredicate::Integer(entry_check) = narrow.predicate.as_ref()? else { return None; };
    let CheckerPredicate::Integer(failure_check) = require.predicate.as_ref()? else { return None; };
    if entry_check.binding != failure_check.binding {
        return None;
    }
    let [parameter] = calls.boundary.parameters.as_slice() else { return None; };
    let [formal] = calls.boundary.checker_parameter_bindings.as_deref()? else { return None; };
    let [(failure_axis, failure_type)] = failure.axes.as_slice() else { return None; };
    if *failure_axis != formal || *failure_type != &parameter.ty {
        return None;
    }
    let (destination, replacement, value) = checked_literal_assignment(calls, group)?;
    let [row] = failure.state.0.as_slice() else { return None; };
    let [current] = row.as_slice() else { return None; };
    if destination != formal || &entry_check.binding != formal
        || replacement != &parameter.ty || entry_check.ty != parameter.ty || failure_check.ty != parameter.ty
        || entry_check.operator != BinaryOperator::Less || failure_check.operator != BinaryOperator::Less
        || value < &failure_check.bound || current.singleton_value()? != value
    {
        return None;
    }
    let (lower, mut upper) = super::typecheck::integer_bounds(&parameter.ty)?;
    if entry_check.bound <= lower {
        return None;
    }
    if entry_check.bound <= upper {
        upper = &entry_check.bound - 1u8;
    }
    Some((parameter.ty.clone(), lower, upper, value.clone()))
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
fn affine_class_composes_and_refines_from_non_anchor_members() {
    let domain = IntegerSet::from_type(&ScalarType::U8).unwrap()
        .keep(&BinaryOperator::LessEqual, &4.into()).unwrap();
    let reflection = Rc::new(IntegerAffineTransform { reflected: true, offset: 4.into() });
    let mut row = IntegerRow::new(vec![Rc::new(domain)]);
    row.test_append_affine(0, Rc::clone(&reflection)).unwrap();
    row.test_append_affine(1, Rc::new(IntegerAffineTransform { reflected: false, offset: 1.into() })).unwrap();
    row.test_append_affine(1, reflection).unwrap();
    assert_eq!(row.canonical_axis(3), 0);
    let mut rows = IntegerInputs(vec![row]);
    assert!(rows.includes_signed(&[3i128, 1, 2, 3]));
    assert!(!rows.includes_signed(&[3i128, 2, 2, 3]));
    rows.restrict_axis(2, &IntegerSet::singleton(&2.into()));
    for (axis, value) in [3, 1, 2, 3].into_iter().enumerate() {
        assert_eq!(rows.singleton_at(axis), Some(&BigInt::from(value)));
    }
    rows.0[0].truncate(2);
    assert_eq!(rows.0[0].affine.as_ref().unwrap().members.len(), 1);
    rows.0[0].truncate(1);
    assert!(rows.0[0].affine.is_none());
}

#[test]
fn affine_class_cap_allows_identity_alias_and_releases_truncated_members() {
    let domain = IntegerSet::from_type(&ScalarType::U8).unwrap()
        .keep(&BinaryOperator::LessEqual, &4.into()).unwrap();
    let mut row = IntegerRow::new(vec![Rc::new(domain)]);
    for index in 1..=MAX_AFFINE_MEMBERS {
        row.test_append_affine(0, Rc::new(IntegerAffineTransform { reflected: false, offset: index.into() })).unwrap();
    }
    assert_eq!(row.affine.as_ref().unwrap().members.len(), MAX_AFFINE_MEMBERS);
    let next = Rc::new(IntegerAffineTransform { reflected: false, offset: (MAX_AFFINE_MEMBERS + 1).into() });
    assert_eq!(row.test_append_affine(0, Rc::clone(&next)), None);
    let cancel = Rc::new(IntegerAffineTransform { reflected: false, offset: -BigInt::from(MAX_AFFINE_MEMBERS) });
    row.test_append_affine(MAX_AFFINE_MEMBERS, cancel).unwrap();
    assert_eq!(row.canonical_axis(MAX_AFFINE_MEMBERS + 1), 0);
    row.replace_domain(MAX_AFFINE_MEMBERS, Rc::new(IntegerSet::singleton(&(MAX_AFFINE_MEMBERS + 2).into())));
    assert_eq!(row[0].singleton_value(), Some(&2.into()));
    assert_eq!(row[MAX_AFFINE_MEMBERS + 1].singleton_value(), Some(&2.into()));
    row.truncate(MAX_AFFINE_MEMBERS);
    assert_eq!(row.affine.as_ref().unwrap().members.len(), MAX_AFFINE_MEMBERS - 1);
    assert_eq!(row.test_append_affine(0, next), Some(()));
}

#[cfg(feature = "internal-checker")]
#[test]
fn typed_bool_domain_is_exact_and_only_available_to_ordinary_ingress() {
    assert!(IntegerSet::from_type(&ScalarType::Bool).is_none());
    assert!(ordinary_parameter_set(&ScalarType::Bool, false).is_none());
    let ordinary = ordinary_parameter_set(&ScalarType::Bool, true).unwrap();
    assert_eq!(ordinary.extremes(), Some((&BigInt::from(0u8), &BigInt::from(1u8))));
    assert!(!ordinary.contains(&BigInt::from(2u8)));
}

#[cfg(feature = "internal-checker")]
#[test]
fn independent_layout_splits_exactly_at_signed_and_unsigned_extrema() {
    let relations = [BinaryOperator::Less, BinaryOperator::LessEqual, BinaryOperator::Equal,
        BinaryOperator::NotEqual, BinaryOperator::GreaterEqual, BinaryOperator::Greater];
    for (ty, min, max, small_min, small_max) in [
        (ScalarType::I8, -128, 127, -128, -127), (ScalarType::U8, 0, 255, 254, 255),
    ] {
        let broad = IntegerSet::from_type(&ty).unwrap();
        let small = broad.keep(&BinaryOperator::GreaterEqual, &small_min.into()).unwrap()
            .keep(&BinaryOperator::LessEqual, &small_max.into()).unwrap();
        let paths = IntegerInputs(vec![IntegerRow::new(vec![Rc::new(small), Rc::new(broad)])]);
        for op in &relations {
            let relation = IntegerRelation::from_operator(op).unwrap();
            let mut work = 0;
            let [no, yes] = paths.ordinary_comparison([0, 1], relation, [true, true], &mut work).unwrap();
            for a in small_min..=small_max {
                for b in min..=max {
                    let values = [i128::from(a), i128::from(b)];
                    let expected = relation.accepts(a.cmp(&b));
                    assert_eq!(yes.includes_signed(&values), expected);
                    assert_eq!(no.includes_signed(&values), !expected);
                }
            }
        }
    }
}

#[cfg(feature = "internal-checker")]
#[test]
fn independent_domain_cardinality_short_circuits_at_the_admission_cap() {
    let broad = IntegerSet::from_type(&ScalarType::I128).unwrap();
    let safe = broad.keep(&BinaryOperator::GreaterEqual, &(-8).into()).unwrap()
        .keep(&BinaryOperator::LessEqual, &7.into()).unwrap();
    let oversized = broad.keep(&BinaryOperator::GreaterEqual, &(-8).into()).unwrap()
        .keep(&BinaryOperator::LessEqual, &8.into()).unwrap();
    let mut work = 0;
    assert_eq!(comparison_domain_size(&safe, MAX_INDEPENDENT_COMPARISON_POINTS, &mut work), Some(Some(16)));
    assert_eq!(comparison_domain_size(&oversized, MAX_INDEPENDENT_COMPARISON_POINTS, &mut work), Some(None));
    assert_eq!(comparison_domain_size(&broad, MAX_INDEPENDENT_COMPARISON_POINTS, &mut work), Some(None));
}

#[cfg(feature = "internal-checker")]
#[test]
fn direct_guard_receipts_restrict_existing_bool_axes_without_a_temp_cell() {
    let broad = Rc::new(IntegerSet::ordinary_bool_domain());
    let mut row = IntegerRow::new(vec![Rc::clone(&broad)]);
    row.append_alias(0, broad);
    let mut output = ScalarOutputRows::Guard { lanes: [IntegerInputs::default(), IntegerInputs::default()], work: 0 };
    output.push_alias(row, 1).unwrap();
    let singleton = Rc::new(IntegerSet::singleton(&BigInt::from(1u8)));
    output.push_alias(IntegerRow::new(vec![Rc::clone(&singleton)]), 0).unwrap();
    let ScalarOutputRows::Guard { lanes: [no, yes], .. } = output else { unreachable!() };
    assert!(no.includes(&[0, 0]) && yes.includes(&[1, 1]));
    assert!(!no.includes(&[0, 1]) && !yes.includes(&[1, 0]));
    assert_eq!(yes.0[1].len(), 1);
    assert!(Rc::ptr_eq(&yes.0[1][0], &singleton));
}

#[cfg(feature = "internal-checker")]
#[test]
fn direct_guard_cursor_requests_only_reached_ordinary_calls() {
    use super::reference::CheckerOrdinaryBoolean as Boolean;
    let name = OccurrenceOrdinal(3);
    let call = OccurrenceOrdinal(7);
    let guard = |enabled| OrdinaryGuard {
        condition: name,
        next: Some(Boolean::And(Box::new([Boolean::Constant(enabled), Boolean::Call(call)]))),
        frames: Vec::new(),
        returned: None,
    };
    let mut roots = IntegerRoots { ordinary_comparison_work: Some(0), ..IntegerRoots::default() };
    let mut path = IntegerInputs::closed();
    let Some(GuardProgress::Done([no, yes])) = guard(false).advance(&mut path, &mut roots) else {
        panic!("dead call arm must finish without suspension");
    };
    assert!(!no.is_unreachable() && yes.is_unreachable());

    let mut path = IntegerInputs::closed();
    let mut reached = guard(true);
    assert!(matches!(reached.advance(&mut path, &mut roots), Some(GuardProgress::Call(actual)) if actual == call));
    assert!(!path.is_unreachable());
    reached.returned = Some([IntegerInputs::default(), std::mem::take(&mut path)]);
    let Some(GuardProgress::Done([no, yes])) = reached.advance(&mut path, &mut roots) else {
        panic!("returned truth rows must complete the saved compound");
    };
    assert!(no.is_unreachable() && !yes.is_unreachable());
}

#[cfg(feature = "internal-checker")]
#[test]
fn ordinary_compounds_only_evaluate_reached_rhs() {
    use super::reference::CheckerOrdinaryBoolean as Boolean;
    let input = IntegerInputs::closed();
    let roots = IntegerRoots::default();
    let mut work = 0;
    let dead_or = Boolean::Or(Box::new([Boolean::Constant(true), Boolean::Opaque]));
    let [false_lane, true_lane] = input.ordinary_boolean(&dead_or, &roots, [true, true], &mut work).unwrap();
    assert!(false_lane.is_unreachable());
    assert!(!true_lane.is_unreachable());
    let dead_and = Boolean::And(Box::new([Boolean::Constant(false), Boolean::Opaque]));
    let [false_lane, true_lane] = input.ordinary_boolean(&dead_and, &roots, [true, false], &mut work).unwrap();
    assert!(!false_lane.is_unreachable());
    assert!(true_lane.is_unreachable());
    let reached = Boolean::Not(Box::new(Boolean::Or(Box::new([Boolean::Constant(false), Boolean::Opaque]))));
    assert!(input.ordinary_boolean(&reached, &roots, [false, true], &mut work).is_none());
    let mut exhausted = MAX_PARTITION_TRANSFER_WORK;
    assert!(input.ordinary_boolean(&dead_or, &roots, [true, true], &mut exhausted).is_none());
}

#[cfg(feature = "internal-checker")]
#[test]
fn bounded_comparison_witness_aliases_respect_shared_and_distinct_rows() {
    let value = Rc::new(IntegerSet::singleton(&5.into()));
    let mut common = IntegerRow::new(vec![Rc::clone(&value)]);
    common.append_alias(0, Rc::clone(&value));
    common.append_alias(0, Rc::clone(&value));
    let mut rows = IntegerInputs(vec![common.clone(); 4]);
    let mut work = MAX_PARTITION_TRANSFER_WORK - 6;
    assert_eq!(rows.common_equal_axes_bounded(&mut work), Some(vec![(1, 0), (2, 0)]));
    assert_eq!(work, MAX_PARTITION_TRANSFER_WORK);
    common.overwrite(2, value, false);
    rows.0.push(common);
    assert_eq!(rows.common_equal_axes_bounded(&mut work), None);
    let mut work = 0;
    assert_eq!(rows.common_equal_axes_bounded(&mut work), Some(vec![(1, 0)]));
}

#[cfg(feature = "internal-checker")]
#[test]
fn ordinary_lexical_comparison_all_relations_match_odd_even_signed_and_unsigned_points() {
    let cases = [(ScalarType::I8, -4, 1, -3), (ScalarType::I8, -4, 1, -2),
        (ScalarType::U8, 0, 5, 5), (ScalarType::U8, 0, 4, 4)];
    let relations = [BinaryOperator::Less, BinaryOperator::LessEqual, BinaryOperator::Equal,
        BinaryOperator::NotEqual, BinaryOperator::GreaterEqual, BinaryOperator::Greater];
    for (ty, min, max, offset) in cases {
        let domain = IntegerSet::from_type(&ty).unwrap()
            .keep(&BinaryOperator::GreaterEqual, &min.into()).unwrap()
            .keep(&BinaryOperator::LessEqual, &max.into()).unwrap();
        let mut row = IntegerRow::new(vec![Rc::new(domain)]);
        row.test_append_affine(0, Rc::new(IntegerAffineTransform { reflected: true, offset: offset.into() })).unwrap();
        let input = IntegerInputs(vec![row]);
        for operator in &relations {
            let relation = IntegerRelation::from_operator(operator).unwrap();
            for (axes, reversed) in [([0, 1], false), ([1, 0], true)] {
                let mut work = 0;
                let [not, yes] = input.ordinary_comparison(axes, relation, [true, true], &mut work).unwrap();
                for x in min..=max {
                    let y = offset - x;
                    let pair = [i128::from(x), i128::from(y)];
                    let expected = relation.accepts(if reversed { y.cmp(&x) } else { x.cmp(&y) });
                    assert_eq!(yes.includes_signed(&pair), expected);
                    assert_eq!(not.includes_signed(&pair), !expected);
                }
            }
        }
    }
}

#[cfg(feature = "internal-checker")]
#[test]
fn affine_overwrite_reanchors_a_promoted_old_anchor_behind_members() {
    let domain = IntegerSet::from_type(&ScalarType::U8).unwrap()
        .keep(&BinaryOperator::LessEqual, &4.into()).unwrap();
    let mut row = IntegerRow::new(vec![Rc::new(domain)]);
    row.test_append_affine(0, Rc::new(IntegerAffineTransform { reflected: true, offset: 4.into() })).unwrap();
    row.test_append_affine(1, Rc::new(IntegerAffineTransform { reflected: false, offset: 1.into() })).unwrap();
    row.append_alias(0, Rc::clone(&row[0]));
    let mut inputs = IntegerInputs(vec![row]);
    let mut work = 0;
    inputs.overwrite_affine(0, &Rc::new(IntegerSet::singleton(&7.into())), true, &mut work).unwrap();
    assert_eq!(inputs.0[0].affine.as_ref().unwrap().anchor, 1);
    assert_eq!(inputs.0[0].entry.as_deref(), Some(&ProjectedEntry::Linked(3)));
    inputs.restrict_axis(2, &IntegerSet::singleton(&2.into()));
    assert!(inputs.includes_signed(&[7, 1, 2, 3]));
    assert!(!inputs.includes_signed(&[7, 1, 2, 0]));
    assert_eq!(inputs.0[0].entry_domain(0).singleton_value(), Some(&3.into()));
    inputs.0[0].truncate(2);
    assert!(inputs.0[0].affine.is_none());
    assert_eq!(inputs.0[0].entry_domain(0).singleton_value(), Some(&3.into()));
}

#[cfg(feature = "internal-checker")]
#[test]
fn affine_overwrite_fails_closed_without_a_projected_snapshot_or_budget() {
    let domain = IntegerSet::from_type(&ScalarType::U8).unwrap()
        .keep(&BinaryOperator::LessEqual, &4.into()).unwrap();
    let mut row = IntegerRow::new(vec![Rc::new(domain)]);
    row.test_append_affine(0, Rc::new(IntegerAffineTransform { reflected: true, offset: 4.into() })).unwrap();
    let mut available = 0;
    assert!(row.prepare_affine_overwrite(0, true, &mut available).is_none());
    let mut exhausted = MAX_PARTITION_TRANSFER_WORK;
    assert!(row.prepare_affine_overwrite(0, false, &mut exhausted).is_none());
    assert_eq!(row.affine.as_ref().unwrap().anchor, 0);
    assert!(row.entry.is_none());
}

#[test]
fn affine_class_pair_receipts_do_not_overwrite_a_mismatched_dependent_domain() {
    let mut row = IntegerRow::new(vec![Rc::new(IntegerSet::from_type(&ScalarType::I8).unwrap())]);
    let transform = Rc::new(IntegerAffineTransform { reflected: true, offset: (-1).into() });
    row.test_append_affine(0, transform).unwrap();
    let set = |number: i8| Rc::new(IntegerSet::singleton(&number.into()));
    let mut inconsistent = row.clone();
    assert_eq!(inconsistent.replace_pair([1, 0], [set(2), set(3)]), None);
    assert_eq!(row.replace_pair([1, 0], [set(2), set(-3)]), Some(()));
    assert_eq!(row[0], set(-3));
    assert_eq!(row[1], set(2));
}

#[test]
fn affine_disjunction_preflights_three_piece_require_witness() {
    let [zero, one, two, three, four, five] = [0, 1, 2, 3, 4, 5].map(BigInt::from);
    let equal = IntegerRelation::from_operator(&BinaryOperator::Equal).unwrap();
    let cuts = IntegerCutUnion::new((equal, &one), (equal, &three), (&zero, &five));
    assert_eq!(cuts.max_added_intervals(), 2);
    let domain = IntegerSet::from_type(&ScalarType::U8).unwrap().keep(&BinaryOperator::LessEqual, &four).unwrap();
    let mut row: IntegerRow = std::iter::once(Rc::new(domain)).collect();
    row.test_append_affine(0, Rc::new(IntegerAffineTransform { reflected: true, offset: four })).unwrap();
    let paths = IntegerInputs(vec![row; 8]);

    let mut work = MAX_PARTITION_TRANSFER_WORK - 8 * (2 + 4);
    assert_eq!(paths.charge_affine_disjunction(1, &cuts, true, &mut work), Some(()));
    assert_eq!(work, MAX_PARTITION_TRANSFER_WORK);
    let witness = paths.disjunction_failure_witness(1, &cuts);
    assert_eq!(witness.0.len(), 8);
    for row in &witness.0 {
        assert_eq!(row[0].interval_count(), 3);
        assert_eq!(row[1].interval_count(), 3);
        assert!(row[0].keep(&BinaryOperator::Equal, &two).unwrap().singleton_value().is_some());
    }
    let mut work = MAX_PARTITION_TRANSFER_WORK - 8 * (2 + 4) + 1;
    assert_eq!(paths.charge_affine_disjunction(1, &cuts, true, &mut work), None);
}

#[test]
fn directive_disjunction_filter_keeps_original_witness_without_building_the_other_side() {
    let [zero, one, nine] = [0, 1, 9].map(BigInt::from);
    let ge = IntegerRelation::from_operator(&BinaryOperator::GreaterEqual).unwrap();
    let less = IntegerRelation::from_operator(&BinaryOperator::Less).unwrap();
    let greater = IntegerRelation::from_operator(&BinaryOperator::Greater).unwrap();
    let all = IntegerCutUnion::new((ge, &zero), (less, &zero), (&zero, &nine));
    let none = IntegerCutUnion::new((less, &zero), (greater, &nine), (&zero, &nine));
    let excluding_one = IntegerCutUnion::new((less, &zero), (greater, &one), (&zero, &nine));
    let axis = Rc::new(IntegerSet::singleton(&one));
    let mut active = IntegerInputs::from_cells(vec![vec![axis.clone()]]);
    let storage = active.0.as_ptr();
    assert!(active.disjunction_failure_witness(0, &all).is_unreachable());
    assert!(active.narrow_disjunction_or_witness(0, &all).is_unreachable());
    assert_eq!(active.0.as_ptr(), storage);
    let witness = active.disjunction_failure_witness(0, &none);
    assert!(Rc::ptr_eq(&witness.0[0][0], &axis));
    assert_eq!(active.0.as_ptr(), storage);
    let witness = active.narrow_disjunction_or_witness(0, &excluding_one);
    assert!(active.is_unreachable());
    assert_eq!(witness.0.as_ptr(), storage);
    assert!(Rc::ptr_eq(&witness.0[0][0], &axis));
    let mut active = IntegerInputs::from_cells(vec![vec![axis.clone()]]);
    let storage = active.0.as_ptr();
    let witness = active.narrow_disjunction_or_witness(0, &none);
    assert!(active.is_unreachable());
    assert_eq!(witness.0.as_ptr(), storage);
    assert!(Rc::ptr_eq(&witness.0[0][0], &axis));
}

#[cfg(feature = "internal-checker")]
#[test]
fn ordered_root_partition_reuses_original_one_sided_rows_and_axes() {
    let [zero, one, two, three, nine] = [0, 1, 2, 3, 9].map(BigInt::from);
    let less = IntegerRelation::from_operator(&BinaryOperator::Less).unwrap();
    let greater = IntegerRelation::from_operator(&BinaryOperator::Greater).unwrap();
    let accepted = IntegerCutUnion::new((less, &one), (greater, &three), (&zero, &nine));
    let at = |point| Rc::new(IntegerSet::singleton(point));
    let outer = at(&zero);
    let other_outer = at(&nine);
    let middle = at(&two);
    let other_middle = at(&three);
    let untouched = at(&nine);
    let passing = IntegerInputs::from_cells(vec![vec![outer.clone(), untouched.clone()], vec![other_outer.clone(), untouched.clone()]]);
    let before = passing.0.as_ptr();
    let (passing, failing) = passing.split_partition(0, &accepted);
    assert!(failing.is_unreachable());
    assert_eq!(passing.0.as_ptr(), before);
    assert!(Rc::ptr_eq(&passing.0[0][0], &outer) && Rc::ptr_eq(&passing.0[1][0], &other_outer));
    let failing = IntegerInputs::from_cells(vec![vec![middle.clone(), untouched.clone()], vec![other_middle, untouched.clone()]]);
    let before = failing.0.as_ptr();
    let (passing, failing) = failing.split_partition(0, &accepted);
    assert!(passing.is_unreachable());
    assert_eq!(failing.0.as_ptr(), before);
    assert!(Rc::ptr_eq(&failing.0[0][0], &middle) && Rc::ptr_eq(&failing.0[1][1], &untouched));
    let mixed = IntegerInputs::from_cells(vec![vec![outer.clone(), untouched.clone()], vec![middle.clone(), untouched.clone()]]);
    let before = mixed.0.as_ptr();
    let (passing, failing) = mixed.split_partition(0, &accepted);
    assert_eq!(passing.0.as_ptr(), before);
    assert!(Rc::ptr_eq(&passing.0[0][0], &outer) && Rc::ptr_eq(&failing.0[0][0], &middle));
    assert!(Rc::ptr_eq(&passing.0[0][1], &untouched) && Rc::ptr_eq(&failing.0[0][1], &untouched));
}

#[cfg(test)]
#[test]
fn actual_call_projection_accepts_only_the_same_singleton_in_every_row() {
    let one = BigInt::from(1);
    let two = BigInt::from(2);
    let coordinate = |value| Rc::new(IntegerSet::singleton(value));
    let same = IntegerInputs::from_cells(vec![vec![coordinate(&one)], vec![coordinate(&one)]]);
    assert_eq!(same.singleton_at(0), Some(&one));
    let distinct = IntegerInputs::from_cells(vec![vec![coordinate(&one)], vec![coordinate(&two)]]);
    assert_eq!(distinct.singleton_at(0), None);
    let broad = IntegerInputs::from_cells(vec![vec![Rc::new(IntegerSet::from_type(&ScalarType::U8).unwrap())]]);
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
    let mut input = IntegerInputs::from_cells(vec![
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
        span: super::ByteSpan::new(0, 0),
        value: IntegerReturn::Const(&returned),
        ty: &ScalarType::I16,
        state: IntegerInputs::from_cells(vec![
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
        span: super::ByteSpan::new(0, 0),
        value: IntegerReturn::Alias(&binding), ty: &ScalarType::I16,
        state: IntegerInputs::from_cells(vec![vec![Rc::new(values)]]), axes: vec![&binding],
    };
    let relation = |operator| IntegerRelation::from_operator(&operator).unwrap();
    let zero = BigInt::from(0);
    let minus_one = BigInt::from(-1);
    let show = IntegerShowOutput::Disjunction([(relation(BinaryOperator::Less), zero.clone()), (relation(BinaryOperator::Greater), zero.clone())]);
    let mut allowed = None;
    assert_eq!(alias.shows_output_with_bound(relation(BinaryOperator::Less), &zero, Some(&show), None, &mut allowed), Some(false));
    assert!(allowed.is_none());

    let constant = IntegerReturningPath {
        span: super::ByteSpan::new(0, 0),
        value: IntegerReturn::Const(&minus_one), ty: &ScalarType::I16,
        state: IntegerInputs(vec![IntegerRow::default()]), axes: Vec::new(),
    };
    assert_eq!(constant.shows_output_with_bound(relation(BinaryOperator::Less), &zero, Some(&show), None, &mut allowed), Some(true));
    assert!(allowed.is_none());

    assert_eq!(alias.shows_output_with_bound(relation(BinaryOperator::NotEqual), &zero, Some(&show), None, &mut allowed), Some(true));
    assert!(allowed.is_some());
}

#[test]
fn symbolic_local_overwrite_preserves_the_other_members_of_a_value_class() {
    let old = Rc::new(IntegerSet::from_type(&ScalarType::I8).unwrap());
    let zero = Rc::new(IntegerSet::singleton(&BigInt::from(0)));
    let one = Rc::new(IntegerSet::singleton(&BigInt::from(1)));
    let mut row = IntegerRow::new(vec![Rc::clone(&old), Rc::clone(&old)]);
    row.append_alias(1, Rc::clone(&old));
    row.append_alias(0, Rc::clone(&old));
    row.append_alias(2, Rc::clone(&old));
    row.append_alias(2, Rc::clone(&old));
    let mut detached = row.clone();
    detached.overwrite(2, Rc::clone(&zero), false);
    assert_eq!(detached.equal_axes(), &[(3, 0), (4, 1), (5, 1)]);
    detached.overwrite(1, Rc::clone(&one), false);
    assert_eq!(detached.equal_axes(), &[(3, 0), (5, 4)]);
    detached.replace_domain(5, Rc::clone(&zero));
    assert!(Rc::ptr_eq(&detached[0], &old));
    assert!(Rc::ptr_eq(&detached[1], &one));
    assert!(Rc::ptr_eq(&detached[2], &zero));
    assert!(Rc::ptr_eq(&detached[4], &zero));
    assert!(Rc::ptr_eq(&detached[5], &zero));
    detached.truncate(4);
    assert_eq!(detached.equal_axes(), &[(3, 0)]);
    detached.overwrite(3, zero, false);
    assert!(detached.equal_axes.is_none());
    assert_eq!(row.equal_axes(), &[(2, 1), (3, 0), (4, 1), (5, 1)]);
}

#[test]
fn symbolic_projected_entry_survives_rebinding_and_expired_copies() {
    let old = Rc::new(IntegerSet::from_type(&ScalarType::I8).unwrap());
    let zero = Rc::new(IntegerSet::singleton(&BigInt::from(0)));
    let negative = Rc::new(old.keep(&BinaryOperator::Less, &BigInt::from(0)).unwrap());
    let mut row = IntegerRow::new(vec![Rc::clone(&old)]);
    row.append_alias(0, Rc::clone(&old));
    row.append_alias(1, Rc::clone(&old));
    row.overwrite(0, Rc::clone(&zero), true);
    assert!(row.entry_alias(0, 1) && row.entry_alias(0, 2));
    assert!(!row.entry_alias(0, 0));
    row.overwrite(1, Rc::clone(&zero), false);
    row.replace_domain(2, Rc::clone(&negative));
    assert!(Rc::ptr_eq(row.entry_domain(0), &negative));
    row.truncate(2);
    assert!(Rc::ptr_eq(row.entry_domain(0), &negative));
    assert!(!row.entry_alias(0, 1));
    row.overwrite(0, Rc::clone(&zero), true);
    assert!(Rc::ptr_eq(row.entry_domain(0), &negative));
    assert!(Rc::ptr_eq(&row[0], &zero));
}
