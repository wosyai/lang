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
    NamedBoolEntry { binding: ReferenceBindingId, value: bool },
    #[cfg_attr(not(feature = "internal-checker"), allow(dead_code))]
    ProofReflexiveBranch(bool),
    Integer(IntegerPredicate),
    EntryDomain { binding: ReferenceBindingId, values: IntegerSet },
    DirectiveDisjunction {
        binding: ReferenceBindingId, ty: ScalarType, atoms: [(IntegerRelation, BigInt); 2],
        #[cfg(feature = "internal-checker")]
        operands: [super::ByteSpan; 2],
    },
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

pub(super) use super::typecheck::concrete_integer_literal;

pub(super) struct ImportedLocal<'a> {
    pub(super) position: usize,
    pub(super) values: ImportedIntegerValues<'a>,
    pub(super) guards: Vec<ImportedEntryGuard<'a>>,
}

#[cfg(feature = "internal-checker")]
pub(super) enum ImportedVerifiedReturn<'a> {
    Constant { values: &'a std::cell::OnceCell<Rc<IntegerSet>>, bound: &'a BigInt },
    EntryAlias(usize),
    EntryAffine { slot: usize, transform: &'a Rc<IntegerAffineTransform> },
    UnaryPartitions(&'a [IntegerReturnPartition]),
    PairPartitions(&'a ProjectedIntegerPair),
}

#[derive(Clone, Copy)]
enum ImportedScalarArgument<'a> {
    Literal(&'a BigInt),
    Axis(usize),
}

pub(super) enum ImportedIntegerValues<'a> {
    Ready(IntegerSet, std::marker::PhantomData<&'a ()>),
    #[cfg(feature = "internal-checker")]
    Shared {
        result: ImportedVerifiedReturn<'a>,
        guards: &'a [ImportedEntryGuard<'a>],
        domain: Option<(usize, &'a IntegerSet)>,
        meter_rectangle: bool,
        full_ranked_meter: bool,
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

#[derive(Clone)]
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
    #[cfg(feature = "internal-checker")]
    fn allows_extended_scalar_locals(self) -> bool { false }
    #[cfg(feature = "internal-checker")]
    fn allows_snapshot_history(self) -> bool { false }
    #[cfg(feature = "internal-checker")]
    fn allows_context_history(self) -> bool { false }
    #[cfg(feature = "internal-checker")]
    fn allows_imported_condition_comparisons(self) -> bool { false }
    #[cfg(feature = "internal-checker")]
    fn imported_enum_match_position(self) -> Option<usize> { None }
    #[cfg(feature = "internal-checker")]
    fn callable_history(self) -> Option<&'a std::cell::RefCell<CallableHistories<'a>>> { None }
    #[cfg(feature = "internal-checker")]
    fn startup_trace(self) -> Option<(&'a wosy_syntax::SourceIdentity, super::checker_project::failure_search::SharedStartupTrace)> { None }
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
    const OBSERVE_FIXED_ARRAY: bool = false;

    #[cfg(feature = "internal-checker")]
    const OBSERVE_RUNTIME_FIXED_INDEX: bool = false;

    #[cfg(feature = "internal-checker")]
    fn observe_runtime_fixed_index(
        self, _: &'a super::reference::CheckerOrdinaryScalarLocal, _: usize,
    ) -> Option<()> { None }

    #[cfg(feature = "internal-checker")]
    const OBSERVE_RESULT_PAIR: bool = false;

    #[cfg(feature = "internal-checker")]
    fn observe_result_pair(
        self, _: &'a FunctionCallSites, _: BlockOrdinal, _: FinalOutputSlot, _: &IntegerRoots<'a>, _: &IntegerInputs,
    ) -> Option<()> { None }

    #[cfg(feature = "internal-checker")]
    fn take_fixed_array_entries(self) -> Option<(usize, IntegerInputs)> { None }

    #[cfg(feature = "internal-checker")]
    fn observe_fixed_array(
        self, _: &'a FunctionCallSites, _: FinalOutputSlot, _: &ScalarType, _: &IntegerRoots<'a>, _: &IntegerInputs,
    ) -> Option<()> { None }

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
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum CallableHistoryStep {
    Branch(wosy_syntax::ByteSpan, bool),
    ProviderBranch(usize, wosy_syntax::ByteSpan, bool),
    MatchArm(wosy_syntax::ByteSpan),
    ProviderMatchArm(usize, wosy_syntax::ByteSpan),
    Call(wosy_syntax::ByteSpan),
    Return(wosy_syntax::ByteSpan),
    ProviderCall(usize, wosy_syntax::ByteSpan),
    ProviderReturn(usize, wosy_syntax::ByteSpan),
}

#[cfg(feature = "internal-checker")]
struct CallableHistories<'a> {
    nodes: Vec<(usize, CallableHistoryStep)>,
    interned: HashMap<(usize, CallableHistoryStep), usize>,
    work: &'a std::cell::Cell<usize>,
    outgoing_call: Option<Vec<CallableHistoryStep>>,
    scalar_return_steps: Option<Vec<Vec<CallableHistoryStep>>>,
    unit_return_arity: Option<usize>,
    unit_return_paths: Option<Vec<UnitHistoryPiece>>,
    received_unit_paths: Option<Vec<UnitHistoryPiece>>,
    step_owner: Option<usize>,
    stopped: bool,
}

#[cfg(feature = "internal-checker")]
impl CallableHistories<'_> {
    fn charge(&self, amount: usize) -> Option<()> {
        let mut work = self.work.get();
        charge_call_work(&mut work, amount)?;
        self.work.set(work);
        Some(())
    }

    fn boundary(&self, span: wosy_syntax::ByteSpan, returned: bool) -> CallableHistoryStep {
        match (self.step_owner, returned) {
            (None, false) => CallableHistoryStep::Call(span),
            (None, true) => CallableHistoryStep::Return(span),
            (Some(owner), false) => CallableHistoryStep::ProviderCall(owner, span),
            (Some(owner), true) => CallableHistoryStep::ProviderReturn(owner, span),
        }
    }

    fn append(&mut self, row: &mut IntegerRow, step: CallableHistoryStep) {
        let node = (row.callable_history, step);
        row.callable_history = *self.interned.entry(node).or_insert_with(|| {
            self.nodes.push(node);
            self.nodes.len()
        });
    }

    fn append_rows(&mut self, rows: &mut IntegerInputs, span: wosy_syntax::ByteSpan, arm: bool) -> Option<()> {
        self.charge(rows.0.len())?;
        let step = self.step_owner.map_or(CallableHistoryStep::Branch(span, arm),
            |owner| CallableHistoryStep::ProviderBranch(owner, span, arm));
        for row in &mut rows.0 { self.append(row, step); }
        Some(())
    }

    fn append_match_lane(&mut self, rows: &mut IntegerInputs, test: Option<(super::ByteSpan, bool)>, arm: super::ByteSpan) -> Option<()> {
        if let Some((condition, taken)) = test { self.append_rows(rows, condition, taken)?; }
        self.charge(rows.0.len())?;
        let step = self.step_owner.map_or(CallableHistoryStep::MatchArm(arm),
            |owner| CallableHistoryStep::ProviderMatchArm(owner, arm));
        for row in &mut rows.0 { self.append(row, step); }
        Some(())
    }

    fn steps(&self, mut id: usize) -> Option<Vec<CallableHistoryStep>> {
        let mut reversed = Vec::new();
        while id != 0 {
            self.charge(1)?;
            let (parent, step) = *self.nodes.get(id - 1)?;
            reversed.push(step);
            id = parent;
        }
        reversed.reverse();
        Some(reversed)
    }
}

#[cfg(feature = "internal-checker")]
#[derive(Clone, Copy)]
pub(super) enum HistoryFrameEffects { None, Snapshot, LexicalContext }

#[cfg(feature = "internal-checker")]
#[derive(Clone, Copy)]
struct TrackCallableHistory<'a>(&'a std::cell::RefCell<CallableHistories<'a>>, HistoryFrameEffects);

#[cfg(feature = "internal-checker")]
impl<'a> ScopedAuthoredReturn<'a> for TrackCallableHistory<'a> {
    const ENABLED: bool = false;
    fn callable_history(self) -> Option<&'a std::cell::RefCell<CallableHistories<'a>>> { Some(self.0) }
    fn allows_snapshot_history(self) -> bool { matches!(self.1, HistoryFrameEffects::Snapshot) }
    fn allows_context_history(self) -> bool { matches!(self.1, HistoryFrameEffects::LexicalContext) }
    fn at(self, _: &FunctionCallSites, _: BlockOrdinal, _: FinalOutputSlot, _: &ScalarType,
        _: &IntegerRoots<'_>, _: &IntegerInputs) -> Option<&'a BigInt> { None }
}

#[cfg(feature = "internal-checker")]
#[derive(Clone, Copy)]
struct OrderedRootConditions<'a, const EXACT_COMPOUND: bool = false, const ORDINARY_LOCALS: bool = false>(&'a super::ScalarBlock, bool);

#[cfg(feature = "internal-checker")]
impl<'a, const EXACT_COMPOUND: bool, const ORDINARY_LOCALS: bool> ScopedAuthoredReturn<'a>
    for OrderedRootConditions<'a, EXACT_COMPOUND, ORDINARY_LOCALS>
{
    const ENABLED: bool = false;
    const ALLOW_ORDINARY_INTEGER_LOCALS: bool = ORDINARY_LOCALS;
    fn allows_extended_scalar_locals(self) -> bool { ORDINARY_LOCALS && self.1 }

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
    pub(super) witness_outside_domain: bool,
    pub(super) state: IntegerInputs,
    pub(super) axes: Vec<(&'a ReferenceBindingId, &'a ScalarType)>,
}

#[cfg(feature = "internal-checker")]
impl<'a> SymbolicLiteralFailure<'a> {
    fn from_local(
        slot: FinalOutputSlot, roots: &IntegerRoots<'a>, row: &IntegerRow, axis: usize,
        returned: Cow<'_, BigInt>, work: &mut usize,
    ) -> Option<Self> {
        charge_call_work(work, row.len().checked_add(row.equal_axes().len())?.checked_add(2)?)?;
        row.charge_affine_replacement(axis, 1, work)?;
        charge_call_work(work, 0)?;
        let mut preimage = row.clone();
        preimage.replace_domain(axis, Rc::new(IntegerSet::singleton(returned.as_ref())));
        Some(Self::from_row(slot, roots, &preimage, None, returned, None))
    }

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
        Self { output: slot.span, returned: returned.into_owned(), witness_outside_domain: false, state: IntegerInputs::from_cells(vec![state]),
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
    work: usize,
    pending: usize,
    checks: &'observation mut [SymbolicLiteralCheck<'a, 'body>],
}

#[cfg(feature = "internal-checker")]
#[derive(Clone, Copy)]
struct OrderedRootLiteralProofs<'a, 'bound, 'checks, 'observation, const STEP_FIRST: bool, const PURE: bool> {
    conditions: OrderedRootConditions<'a, false, PURE>,
    observed: &'observation std::cell::RefCell<BatchedLiteralEvidence<'bound, 'a, 'checks>>,
}

#[cfg(feature = "internal-checker")]
impl<'a, 'bound, const STEP_FIRST: bool, const PURE: bool> ScopedAuthoredReturn<'a>
    for OrderedRootLiteralProofs<'a, 'bound, '_, '_, STEP_FIRST, PURE> {
    const ENABLED: bool = false;
    const OBSERVE_RETURN: bool = true;
    const ALLOW_ORDINARY_INTEGER_LOCALS: bool = PURE;
    const REQUIRE_TOTAL_RETURN: bool = PURE;

    fn allows_extended_scalar_locals(self) -> bool { PURE }

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
        observe_batched_literal_returns::<STEP_FIRST, PURE>(&mut self.observed.borrow_mut(), slot, value, roots, path)
    }
}

#[cfg(feature = "internal-checker")]
fn observe_batched_literal_returns<'a, 'bound, const STEP_FIRST: bool, const PURE: bool>(
    observed: &mut BatchedLiteralEvidence<'bound, 'a, '_>, slot: FinalOutputSlot, value: IntegerReturn<'a>,
    roots: &IntegerRoots<'a>, path: &IntegerInputs,
) -> Option<()> {
    let axis = if PURE {
        match value {
            IntegerReturn::Const(_) => None,
            IntegerReturn::Alias(binding) => Some(roots.lookup(binding)?.0),
        }
    } else {
        symbolic_return_axis(value, roots)?
    };
    observed.returned = true;
    let BatchedLiteralEvidence { pending, checks, work, .. } = observed;
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
        if PURE { charge_call_work(work, checks.len())?; }
        for check in checks.iter_mut().filter(|check| check.failure.is_none()) {
            if let Some(witness) = rejected_point(check.relation, check.bound) {
                check.failure = Some(match axis {
                    Some(axis) if PURE && axis >= roots.parameters =>
                        SymbolicLiteralFailure::from_local(slot, roots, row, axis, Cow::Borrowed(witness), work)?,
                    _ => SymbolicLiteralFailure::from_row(slot, roots, row, axis, Cow::Borrowed(witness), None),
                });
                *pending -= 1;
            }
        }
        if *pending == 0 || axis.is_none() {
            break;
        }
    }
    Some(())
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
pub(super) enum QuantifiedIntegerReturn<'a> {
    Vacuous,
    Impossible,
    Output(&'a IntegerShowConstraints<'static>),
    Comparison(Option<&'a IntegerShowConstraints<'static>>, IntegerRelation, &'a BigInt),
    Avoiding(Option<&'a IntegerShowConstraints<'static>>, &'a IntegerSet),
    EntryWitness {
        slot: usize, relation: IntegerRelation, domain: &'a IntegerSet, bounds: Option<&'a IntegerShowConstraints<'static>>,
        translation: Option<(&'a BigInt, &'a ScalarType)>,
    },
}

#[cfg(feature = "internal-checker")]
impl<'a> QuantifiedIntegerReturn<'a> {
    pub(super) fn exclusions(self) -> Option<&'a IntegerSet> {
        match self {
            Self::Avoiding(_, excluded) => Some(excluded),
            _ => None,
        }
    }

    pub(super) fn uses_entry(self) -> bool { matches!(self, Self::EntryWitness { .. }) }

    fn witness_value<'point>(
        original: &'point BigInt, translation: Option<(&BigInt, &ScalarType)>, work: &mut usize,
    ) -> Option<Cow<'point, BigInt>> {
        let Some((offset, ty)) = translation else { return Some(Cow::Borrowed(original)); };
        charge_call_work(work, 1)?;
        let translated = original + offset;
        super::typecheck::integer_literal_fits_type(&translated, ty).then_some(Cow::Owned(translated))
    }

    pub(super) fn rejected_at_call(self, returned: &BigInt, arguments: &[&BigInt], work: &mut usize)
        -> Option<Option<bool>>
    {
        if let Self::EntryWitness { slot, relation, domain, bounds, translation } = self {
            if !matches!(arguments.len(), 1 | 2) { return None; }
            let entry = arguments.get(slot)?;
            let depth = (usize::BITS - domain.interval_count().leading_zeros()) as usize + 1;
            charge_call_work(work, depth.checked_add(1)?)?;
            let witness = Self::witness_value(entry, translation, work)?;
            if !domain.covers_closed(witness.as_ref(), witness.as_ref()) { return Some(Some(true)); }
            charge_call_work(work, 1usize.checked_add(bounds.map_or(0, IntegerShowConstraints::interval_lookup_work))?)?;
            return Some((!relation.accepts(returned.cmp(witness.as_ref()))
                || bounds.is_some_and(|bounds| !bounds.holds_value(returned))).then_some(false));
        }
        Some(self.rejected_point(returned, returned, work)?.map(|_| false))
    }

    pub(super) fn rejected_point<'point>(self, low: &'point BigInt, high: &'point BigInt, work: &mut usize) -> Option<Option<&'point BigInt>>
    where Self: 'point,
    {
        let failure = match self {
            Self::EntryWitness { .. } => return None,
            Self::Vacuous => { charge_call_work(work, 1)?; None }
            Self::Impossible => { charge_call_work(work, 1)?; Some(low) }
            Self::Output(bounds) => {
                charge_call_work(work, bounds.interval_lookup_work())?;
                bounds.first_violation_in_interval(low, high)
            }
            Self::Comparison(bounds, relation, bound) => {
                charge_call_work(work, 3 + bounds.map_or(0, IntegerShowConstraints::interval_lookup_work))?;
                let rejected = relation.complement();
                bounds.and_then(|bounds| bounds.first_violation_in_interval(low, high))
                    .or_else(|| rejected.accepts(low.cmp(bound)).then_some(low))
                    .or_else(|| rejected.accepts(high.cmp(bound)).then_some(high))
                    .or_else(|| (rejected.accepts(std::cmp::Ordering::Equal) && low < bound && bound < high).then_some(bound))
            }
            Self::Avoiding(bounds, excluded) => {
                let depth = (usize::BITS - excluded.interval_count().leading_zeros()) as usize + 1;
                charge_call_work(work, depth.checked_add(bounds.map_or(0, IntegerShowConstraints::interval_lookup_work))?)?;
                bounds.and_then(|bounds| bounds.first_violation_in_interval(low, high))
                    .or_else(|| excluded.first_intersection_point(low, high))
            }
        };
        Some(failure)
    }
}

#[cfg(feature = "internal-checker")]
fn rejected_independent_witness_pair<'a>(
    result: &'a IntegerSet, witness: (&'a BigInt, &'a BigInt),
    relation: IntegerRelation, work: &mut usize,
) -> Option<Option<(&'a BigInt, &'a BigInt)>> {
    use std::cmp::Ordering::{Equal, Greater, Less};
    let (low, high) = result.extremes()?;
    charge_call_work(work, 3)?;
    if !relation.accepts(Less) && low < witness.1 { return Some(Some((low, witness.1))); }
    if !relation.accepts(Greater) && high > witness.0 { return Some(Some((high, witness.0))); }
    if !relation.accepts(Equal) {
        let depth = (usize::BITS - result.interval_count().leading_zeros()) as usize + 1;
        charge_call_work(work, depth)?;
        return Some(result.first_intersection_point(witness.0, witness.1).map(|point| (point, point)));
    }
    Some(None)
}

#[cfg(all(test, feature = "internal-checker"))]
#[test]
fn paired_witness_extrema_and_sparse_collision_match_small_point_oracle() {
    let relation = |operator| IntegerRelation::from_operator(&operator).unwrap();
    let never = relation(BinaryOperator::Equal).intersection(relation(BinaryOperator::NotEqual));
    let relations = [
        never, never.complement(), relation(BinaryOperator::Equal), relation(BinaryOperator::NotEqual),
        relation(BinaryOperator::Less), relation(BinaryOperator::LessEqual),
        relation(BinaryOperator::Greater), relation(BinaryOperator::GreaterEqual),
    ];
    for mask in 1u8..32 {
        let points = (-2..=2).zip(0..5).filter(|(_, bit)| mask & (1 << bit) != 0)
            .map(|(value, _)| BigInt::from(value)).collect::<Vec<_>>();
        let result = points.iter().fold(IntegerSet::default(), |set, point| set.join(IntegerSet::singleton(point)));
        for lower in -2..=2 {
            for upper in lower..=2 {
                let (low, high) = (BigInt::from(lower), BigInt::from(upper));
                for relation in relations {
                    let expected = points.iter().any(|point| (lower..=upper).any(|candidate|
                        !relation.accepts(point.cmp(&BigInt::from(candidate)))));
                    let actual = rejected_independent_witness_pair(&result, (&low, &high), relation, &mut 0).unwrap();
                    assert_eq!(actual.is_some(), expected);
                    if let Some((returned, candidate)) = actual {
                        assert!(points.contains(returned) && &low <= candidate && candidate <= &high);
                        assert!(!relation.accepts(returned.cmp(candidate)));
                    }
                }
            }
        }
    }
}

#[cfg(feature = "internal-checker")]
#[derive(Clone, Copy)]
enum OrderedReturnedGoal<'a> {
    Literal(IntegerRelation, &'a BigInt),
    Entry(&'a IntegerAffineTransform),
    Quantified(QuantifiedIntegerReturn<'a>),
}

#[cfg(feature = "internal-checker")]
#[derive(Clone, Copy)]
struct OrderedRootLiteralProof<'a, 'observation, const TOTAL: bool> {
    conditions: OrderedRootConditions<'a, TOTAL>,
    goal: OrderedReturnedGoal<'observation>,
    guard: Option<&'observation LiteralGuardAtReturn<'observation>>,
    imported_binding: Option<&'a ReferenceBindingId>,
    qualified_conditional_return: bool,
    imported_relation_work: Option<&'observation std::cell::RefCell<usize>>,
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
        let axis = match (value, self.imported_binding) {
            // Admission already authenticated the two exact lexical return
            // occurrences. Do not require both to be the final imported slot
            // or collapse distinct lanes before checking their entry relation.
            (IntegerReturn::Alias(binding), Some(_)) if self.qualified_conditional_return =>
                Some(roots.lookup(binding)?.0),
            (IntegerReturn::Alias(binding), Some(expected)) if binding == expected => {
                let (axis, _) = roots.lookup(binding)?;
                if axis < roots.parameters { return None; }
                Some(axis)
            }
            (_, Some(_)) => return None,
            _ => symbolic_return_axis(value, roots)?,
        };
        let mut observed = self.observed.borrow_mut();
        if observed.1.is_some() {
            return Some(());
        }
        if let OrderedReturnedGoal::Entry(expected) = self.goal {
            let axis = axis?;
            let work = &mut *self.imported_relation_work?.borrow_mut();
            let [input] = roots.axes.get(..roots.parameters)? else { return None; };
            let (minimum, maximum) = super::typecheck::integer_bounds(input.1)?;
            for row in &path.0 {
                let domain = row.entry_domain(0);
                charge_call_work(work, row.len().checked_add(domain.interval_count())?.checked_add(1)?)?;
                expected.fits(domain, &minimum, &maximum)?;
                let actual_transform;
                let actual = if row.entry_alias(0, axis) {
                    AxisProofValue::Identity
                } else if let Some(constant) = row.get(axis)?.singleton_value() {
                    AxisProofValue::Literal(constant)
                } else {
                    actual_transform = row.entry_affine(0, axis)?;
                    AxisProofValue::Affine(&actual_transform)
                };
                observed.0 = true;
                if let Some((violated, returned)) = rejected_entry_relationship(domain, actual, expected, work)? {
                    let witness = violated.extremes()?.0;
                    observed.1 = Some(SymbolicLiteralFailure::from_row(
                        slot, roots, row, None, Cow::Owned(returned), Some((0, witness)),
                    ));
                    break;
                }
            }
            return Some(());
        }
        if let OrderedReturnedGoal::Quantified(claim) = self.goal {
            let work = &mut *self.imported_relation_work?.borrow_mut();
            for row in &path.0 {
                observed.0 = true;
                if let QuantifiedIntegerReturn::EntryWitness { slot: witness_slot, relation, domain, bounds, translation } = claim {
                    if !matches!(roots.parameters, 1 | 2) || witness_slot >= roots.parameters { return None; }
                    if roots.parameters == 2 && (row.entry.is_some() || row.affine.is_some() || row.projected_from != 0
                        || !row.equal_axes().is_empty())
                    {
                        // Do not Cartesian-project a relational/imported row.
                        return None;
                    }
                    let independent = axis.filter(|&axis| !row.entry_alias(witness_slot, axis));
                    if independent.is_some() && roots.parameters != 2 { return None; }
                    let independent_result = match independent { Some(axis) => Some(row.get(axis)?), None => None };
                    let entry = row.entry_domain(witness_slot);
                    let bounds_violation = if let (Some(result), Some(bounds)) = (independent_result, bounds) {
                        let mut bad = None;
                        for (low, high) in result.intervals() {
                            charge_call_work(work, bounds.interval_lookup_work())?;
                            bad = bounds.first_violation_in_interval(low, high);
                            if bad.is_some() { break; }
                        }
                        bad
                    } else { None };
                    let depth = (usize::BITS - domain.interval_count().leading_zeros()) as usize + 1;
                    for (low, high) in entry.intervals() {
                        charge_call_work(work, depth.checked_add(1)?)?;
                        let witness_low = QuantifiedIntegerReturn::witness_value(low, translation, work)?;
                        let witness_high = QuantifiedIntegerReturn::witness_value(high, translation, work)?;
                        let outside = domain.first_uncovered_in_interval(witness_low.as_ref(), witness_high.as_ref());
                        if translation.is_some() { charge_call_work(work, 1)?; }
                        let inversion = |witness: &BigInt| translation.map_or_else(|| witness.clone(), |(offset, _)| witness - offset);
                        if let Some(result) = independent_result {
                            let rejected = if let Some(point) = &outside {
                                Some((result.extremes()?.0, point))
                            } else if let Some(returned) = bounds_violation {
                                Some((returned, witness_low.as_ref()))
                            } else {
                                rejected_independent_witness_pair(result, (witness_low.as_ref(), witness_high.as_ref()),
                                    relation, work)?
                            };
                            if let Some((returned, witness)) = rejected {
                                charge_call_work(work, roots.parameters.checked_add(1)?)?;
                                let point = inversion(witness);
                                let mut failure = SymbolicLiteralFailure::from_row(
                                    slot, roots, row, independent, Cow::Borrowed(returned), Some((witness_slot, &point)),
                                );
                                failure.witness_outside_domain = outside.is_some();
                                observed.1 = Some(failure);
                                break;
                            }
                            continue;
                        }
                        let point = if let Some(point) = &outside {
                            Some(Cow::Owned(inversion(point)))
                        } else {
                            charge_call_work(work, 3usize.checked_add(bounds.map_or(0, IntegerShowConstraints::interval_lookup_work))?)?;
                            match value {
                                IntegerReturn::Const(returned) => {
                                    if bounds.is_some_and(|bounds| !bounds.holds_value(returned)) {
                                        Some(Cow::Borrowed(low))
                                    } else {
                                        let rejected = relation.complement();
                                        [(low, &witness_low), (high, &witness_high)].into_iter()
                                            .find_map(|(point, witness)| rejected.accepts(returned.cmp(witness.as_ref())).then_some(Cow::Borrowed(point)))
                                            .or_else(|| (witness_low.as_ref() < returned && returned < witness_high.as_ref()
                                                && rejected.accepts(std::cmp::Ordering::Equal)).then(|| Cow::Owned(inversion(returned))))
                                    }
                                }
                                IntegerReturn::Alias(_) => {
                                    // On the same entry axis, x cmp (x+offset)
                                    // is constant for every checked value.
                                    if !relation.accepts(low.cmp(witness_low.as_ref())) { Some(Cow::Borrowed(low)) }
                                    else { bounds.and_then(|bounds| bounds.first_violation_in_interval(low, high)).map(Cow::Borrowed) }
                                }
                            }
                        };
                        if let Some(point) = point {
                            charge_call_work(work, roots.parameters.checked_add(1)?)?;
                            let returned = match value { IntegerReturn::Const(returned) => returned, IntegerReturn::Alias(_) => point.as_ref() };
                            let mut failure = SymbolicLiteralFailure::from_row(slot, roots, row, None, Cow::Borrowed(returned), Some((witness_slot, point.as_ref())));
                            failure.witness_outside_domain = outside.is_some();
                            observed.1 = Some(failure);
                            break;
                        }
                    }
                    if observed.1.is_some() { break; }
                    continue;
                }
                let witness = match (value, axis) {
                    (IntegerReturn::Const(value), None) => claim.rejected_point(value, value, work)?,
                    (IntegerReturn::Alias(_), Some(axis)) => {
                        let mut found = None;
                        for (low, high) in row.get(axis)?.intervals() {
                            if let Some(point) = claim.rejected_point(low, high, work)? {
                                found = Some(point);
                                break;
                            }
                        }
                        found
                    }
                    _ => return None,
                };
                if let Some(point) = witness {
                    charge_call_work(work, roots.parameters.checked_add(1)?)?;
                    observed.1 = Some(SymbolicLiteralFailure::from_row(slot, roots, row, axis, Cow::Owned(point.clone()), None));
                    break;
                }
            }
            return Some(());
        }
        let OrderedReturnedGoal::Literal(relation, bound) = self.goal else { return None; };
        let rejected = relation.complement();
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
                (IntegerReturn::Const(value), _) => rejected.accepts(value.cmp(bound)).then_some(value),
                (IntegerReturn::Alias(_), Some(axis)) if axis >= roots.parameters => {
                    let result_domain = row.get(axis)?;
                    if let Some(work) = self.imported_relation_work {
                        charge_call_work(&mut work.borrow_mut(), result_domain.interval_count().checked_add(1)?)?;
                        result_domain.first_comparison_point(rejected, bound)
                    } else {
                        let returned = result_domain.singleton_value()?;
                        rejected.accepts(returned.cmp(bound)).then_some(returned)
                    }
                }
                (IntegerReturn::Alias(_), Some(axis)) => row.get(axis)?.first_comparison_point(rejected, bound),
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
            // The return is reported separately; the diagnostic state still
            // contains only the callable's entry coordinates.
            let entry_axis = axis.filter(|&axis| axis < roots.parameters);
            observed.1 = Some(match (self.imported_relation_work, axis) {
                (Some(work), Some(local)) if local >= roots.parameters =>
                    SymbolicLiteralFailure::from_local(slot, roots, row, local, returned, &mut work.borrow_mut())?,
                _ => SymbolicLiteralFailure::from_row(slot, roots, row, entry_axis, returned, entry),
            });
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
                (ScalarExpression::Name { .. }, ScalarExpression::Name { .. }, BinaryOperator::Add | BinaryOperator::WrappingAdd | BinaryOperator::Subtract) => {
                    let left_axis = formal_axis(left_ordinal)?;
                    let right_axis = formal_axis(right_ordinal)?;
                    if left_axis == right_axis && !matches!(operator, BinaryOperator::WrappingAdd) {
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
                (ScalarExpression::Name { .. }, literal, BinaryOperator::Add | BinaryOperator::WrappingAdd | BinaryOperator::Subtract) =>
                    (exact_formal(left_ordinal, literal)?, concrete_integer_literal(literal)?),
                (literal, ScalarExpression::Name { .. }, BinaryOperator::Add | BinaryOperator::WrappingAdd) => {
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
            let value = operation.apply(left, right, &min, &max)?;
            fits(&value).then_some(value)
        }).as_ref()
    }
}

#[cfg(feature = "internal-checker")]
#[derive(Clone, Copy)]
enum AuthoredIntegerOperation {
    Add,
    WrappingAdd,
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
            BinaryOperator::WrappingAdd => Self::WrappingAdd,
            BinaryOperator::Subtract => Self::Subtract,
            BinaryOperator::Multiply => Self::Multiply,
            BinaryOperator::Divide => Self::Divide,
            BinaryOperator::Remainder => Self::Remainder,
            _ => return None,
        })
    }

    fn apply(self, left: Cow<'_, BigInt>, right: Cow<'_, BigInt>, min: &BigInt, max: &BigInt) -> Option<BigInt> {
        Some(match (self, left, right) {
            (Self::Add, Cow::Owned(left), Cow::Owned(right)) => left + right,
            (Self::Add, Cow::Owned(left), right) => left + right.as_ref(),
            (Self::Add, left, Cow::Owned(right)) => right + left.as_ref(),
            (Self::Add, left, right) => left.as_ref() + right.as_ref(),
            (Self::WrappingAdd, left, right) =>
                super::typecheck::wrapping_integer_sum(left.as_ref(), right.as_ref(), min, max)?,
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
                    let computed = operation.apply(left, value, min, max)?;
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
pub(super) struct IntegerAffineTransform {
    reflected: bool,
    offset: BigInt,
}

impl IntegerAffineTransform {
    #[cfg(feature = "internal-checker")]
    pub(super) fn unit(reflected: bool, offset: BigInt) -> Self {
        Self { reflected, offset }
    }

    fn apply(&self, domain: &IntegerSet, inverse: bool) -> IntegerSet {
        domain.affine_unit(self.reflected, &self.offset, inverse)
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn apply_point(&self, input: &BigInt) -> BigInt {
        if self.reflected { &self.offset - input } else { input + &self.offset }
    }

    #[cfg(feature = "internal-checker")]
    fn integral_reflection_fixed_point(&self) -> Option<BigInt> {
        (self.reflected && !self.offset.bit(0)).then(|| &self.offset >> 1usize)
    }

    #[cfg(feature = "internal-checker")]
    fn pull_back_cut(&self, relation: IntegerRelation, bound: &BigInt) -> (IntegerRelation, BigInt) {
        if self.reflected {
            (relation.reversed(), &self.offset - bound)
        } else {
            (relation, bound - &self.offset)
        }
    }

    #[cfg(feature = "internal-checker")]
    fn identity_everywhere(&self) -> bool {
        !self.reflected && self.offset.sign() == num_bigint::Sign::NoSign
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

#[cfg(all(test, feature = "internal-checker"))]
#[test]
fn signed_affine_reflection_has_only_its_integral_fixed_point() {
    for (offset, fixed) in [(-256, Some(-128)), (-255, None), (-2, Some(-1)), (-1, None),
        (0, Some(0)), (254, Some(127)), (255, None), (510, Some(255))]
    {
        let transform = IntegerAffineTransform { reflected: true, offset: BigInt::from(offset) };
        assert_eq!(transform.integral_reflection_fixed_point(), fixed.map(BigInt::from));
        if let Some(fixed) = fixed.map(BigInt::from) {
            assert_eq!(transform.apply_point(&fixed), fixed);
        }
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
    #[cfg(feature = "internal-checker")]
    callable_history: usize,
}

impl IntegerRow {
    fn new(cells: Vec<Rc<IntegerSet>>) -> Self { Self { cells, equal_axes: None, affine: None, entry: None, projected_from: 0,
        #[cfg(feature = "internal-checker")]
        callable_history: 0,
    } }

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
    #[cfg(feature = "internal-checker")]
    fn require_independent_named_pair(&self, work: &mut usize) -> Option<()> {
        let lookup = (usize::BITS - self.equal_axes().len().leading_zeros()) as usize;
        charge_call_work(work, lookup.checked_mul(2)?.checked_add(MAX_AFFINE_MEMBERS + 1)?)?;
        let left = self.canonical_axis(0);
        let right = self.canonical_axis(1);
        (self.entry.is_none() && left != right
            && !self.affine.as_ref().is_some_and(|class| class.contains(left) && class.contains(right))).then_some(())
    }

    #[cfg(feature = "internal-checker")]
    fn require_named_pair_entry(&self, bool_axis: Option<usize>, work: &mut usize) -> Option<()> {
        self.require_independent_named_pair(work)?;
        if let Some(axis) = bool_axis {
            charge_call_work(work, 1)?;
            bool_result_lane(self.get(axis)?.singleton_value()?)?;
        }
        Some(())
    }

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
    #[cfg(feature = "internal-checker")]
    symbolic_context_work: Option<usize>,
    #[cfg(feature = "internal-checker")]
    symbolic_context_entry_amount: bool,
    affine_work: Option<usize>,
    #[cfg(feature = "internal-checker")]
    named_pair_rectangles: bool,
    #[cfg(feature = "internal-checker")]
    named_pair_bool_axis: Option<usize>,
    #[cfg(feature = "internal-checker")]
    named_exact_bool: bool,
    #[cfg(feature = "internal-checker")]
    allow_ordinary_integer_locals: bool,
    #[cfg(feature = "internal-checker")]
    allow_ordinary_affine_overwrites: bool,
    #[cfg(feature = "internal-checker")]
    exact_returning_local_writes: bool,
    #[cfg(feature = "internal-checker")]
    projected_narrow_integer: bool,
    #[cfg(feature = "internal-checker")]
    projected_scalar_local_write: bool,
    #[cfg(feature = "internal-checker")]
    allow_ordinary_contextual_places: bool,
    #[cfg(feature = "internal-checker")]
    allow_exact_contextual_bool: bool,
    #[cfg(feature = "internal-checker")]
    allow_exact_contextual_counter_argument: bool,
    #[cfg(feature = "internal-checker")]
    allow_exact_contextual_resource_guard: bool,
    #[cfg(feature = "internal-checker")]
    allow_symbolic_contextual_bool: bool,
    #[cfg(feature = "internal-checker")]
    allow_real_integer_tail_call: bool,
    #[cfg(feature = "internal-checker")]
    allow_extended_scalar_locals: bool,
    #[cfg(feature = "internal-checker")]
    allow_snapshot_history: bool,
    #[cfg(feature = "internal-checker")]
    ordinary_local_work: Option<usize>,
    #[cfg(feature = "internal-checker")]
    ordinary_comparison_work: Option<usize>,
    #[cfg(feature = "internal-checker")]
    startup_trace: Option<(&'a wosy_syntax::SourceIdentity, Rc<std::cell::RefCell<super::checker_project::failure_search::StartUpSearch>>)>,
    #[cfg(feature = "internal-checker")]
    callable_history: Option<&'a std::cell::RefCell<CallableHistories<'a>>>,
    reached_obligation: bool,
    proof_reflexive_branches: bool,
}

impl<'a> IntegerRoots<'a> {
    #[cfg(feature = "internal-checker")]
    fn after_startup_candidate(&self) -> Option<()> {
        if self.startup_trace.as_ref().is_some_and(|(_, trace)| trace.borrow().cannot_continue())
            || self.callable_history.is_some_and(|trace| trace.borrow().stopped) {
            None
        } else { Some(()) }
    }

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

#[cfg(feature = "internal-checker")]
#[derive(Clone, Copy)]
enum SnapshotIndexValue {
    Literal(usize),
    Axis(usize),
}

impl IntegerInputs {
    #[cfg(feature = "internal-checker")]
    fn append_ordinary_copy(&mut self, axis: usize, roots: &mut IntegerRoots<'_>) -> Option<()> {
        for row in &self.0 {
            let work = if row.affine.is_some() {
                roots.affine_work.as_mut()?
            } else {
                roots.ordinary_local_work.get_or_insert(0)
            };
            charge_call_work(work, row.equal_axes().len().checked_add(2)?)?;
        }
        for row in &mut self.0 {
            row.append_alias(axis, Rc::clone(&row[axis]));
        }
        Some(())
    }

    #[cfg(feature = "internal-checker")]
    fn bounded_snapshot_path(
        self, root: &super::checker_directive::CheckerSnapshotNode,
        steps: &[(usize, SnapshotIndexValue)], mut field_axis: impl FnMut(usize) -> Option<usize>,
        work: &mut usize,
    ) -> Option<Self> {
        use super::checker_directive::CheckerSnapshotNode as Node;
        if !(1..=8).contains(&steps.len()) { return None; }
        let mut axes = [None; 64];
        let mut axis_at = |leaf: usize| {
            let slot = axes.get_mut(leaf)?;
            if slot.is_none() { *slot = Some(field_axis(leaf)?); }
            *slot
        };
        fn visit(
            mut row: IntegerRow, node: &Node, steps: &[(usize, SnapshotIndexValue)],
            axis_at: &mut impl FnMut(usize) -> Option<usize>, output: &mut Vec<IntegerRow>,
            work: &mut usize,
        ) -> Option<()> {
            charge_call_work(work, 1)?;
            let Some((&(length, index), rest)) = steps.split_first() else {
                let Node::Leaf(leaf) = node else { return None; };
                let growth = if row.len() == row.cells.capacity() { row.len() } else { 0 };
                charge_call_work(work, row.equal_axes().len().checked_add(growth)?.checked_add(2)?)?;
                let field = axis_at(*leaf)?;
                row.append_alias(field, row.get(field)?.clone());
                output.push(row);
                return Some(());
            };
            let Node::Array(children) = node else { return None; };
            if !(1..=64).contains(&length) || length != children.len() { return None; }
            let axis = match index {
                SnapshotIndexValue::Literal(index) =>
                    return visit(row, children.get(index)?, rest, axis_at, output, work),
                SnapshotIndexValue::Axis(axis) => axis,
            };
            let indices = row.get(axis)?;
            let (lower, upper) = indices.extremes()?;
            let first = usize::try_from(lower).ok()?;
            let last = usize::try_from(upper).ok()?;
            // Do not intersect away invalid indices: every reachable
            // coordinate must be in bounds at this dimension.
            if last >= length || first > last { return None; }
            if first == last {
                return visit(row, children.get(first)?, rest, axis_at, output, work);
            }
            for (lower, upper) in indices.intervals() {
                charge_call_work(work, 1)?;
                for index in usize::try_from(lower).ok()?..=usize::try_from(upper).ok()? {
                    // Charge the copied width and actual equality/affine
                    // preimage propagation before creating the child row.
                    let mut cost = row.len().checked_mul(2)?.checked_add(row.equal_axes().len())?.checked_add(5)?;
                    row.charge_affine_replacement(axis, 1, &mut cost)?;
                    charge_call_work(work, cost)?;
                    let mut split = row.clone();
                    split.replace_domain(axis, Rc::new(IntegerSet::singleton(&BigInt::from(index))));
                    visit(split, children.get(index)?, rest, axis_at, output, work)?;
                }
            }
            Some(())
        }
        let mut result = Vec::new();
        for row in self.0 {
            visit(row, root, steps, &mut axis_at, &mut result, work)?;
        }
        Some(Self(result))
    }

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

    #[cfg(feature = "internal-checker")]
    fn charge_projected_integer_directive(&self, roots: &mut IntegerRoots<'_>, predicate: &CheckerPredicate) -> Option<()> {
        let axis = match predicate {
            CheckerPredicate::Integer(predicate) => Some(roots.lookup(&predicate.binding)?.0),
            CheckerPredicate::Boolean(_) => None,
            _ => return None,
        };
        let work = roots.ordinary_local_work.get_or_insert(0);
        for row in &self.0 {
            // Both directive outcomes may own a row; charge columns, equality
            // copies, and the full interval walk before partitioning.
            let cost = 1usize.checked_add(row.len().checked_mul(2)?)?
                .checked_add(row.equal_axes().len().checked_mul(2)?)?
                .checked_add(axis.map_or(0, |axis| row[axis].interval_count()).checked_mul(2)?)?;
            charge_call_work(work, cost)?;
        }
        Some(())
    }

    fn axis_has_alias(&self, axis: usize) -> bool {
        self.0.iter().any(|row| {
            let canonical = row.canonical_axis(axis);
            row.equal_axes().iter().any(|(_, root)| *root == canonical)
        })
    }

    #[cfg(feature = "internal-checker")]
    fn axis_has_visible_alias(&self, axis: usize, roots: &mut IntegerRoots<'_>) -> Option<bool> {
        let work = roots.ordinary_local_work.get_or_insert(0);
        for row in &self.0 {
            charge_call_work(work, row.equal_axes().len().checked_add(1)?)?;
            let canonical = row.canonical_axis(axis);
            if row.equal_axes().iter().any(|&(alias, root)| root == canonical
                && !matches!(roots.axes[alias].0.kind, super::ReferenceBindingKind::CheckerSnapshotField(_)))
            {
                return Some(true);
            }
        }
        Some(false)
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
    pub(super) fn entry_point(coordinates: &[BigInt]) -> Self {
        Self(vec![coordinates.iter().map(|value| Rc::new(IntegerSet::singleton(value))).collect()])
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

    pub(super) fn project_axis_bounded(&self, axis: usize, work: &mut usize) -> Option<Rc<IntegerSet>> {
        let (first, rest) = self.0.split_first()?;
        let first = &first[axis];
        charge_call_work(work, 1)?;
        let mut remaining = rest.iter().map(|row| &row[axis]);
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
            CheckerPredicate::NamedBoolEntry { .. } | CheckerPredicate::EntryDomain { .. }
            | CheckerPredicate::DirectiveDisjunction { .. } | CheckerPredicate::ProofReflexiveBranch(_) => return None,
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
            CheckerPredicate::NamedBoolEntry { .. } | CheckerPredicate::EntryDomain { .. }
            | CheckerPredicate::DirectiveDisjunction { .. } | CheckerPredicate::ProofReflexiveBranch(_) => return None,
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
    pub(super) fn remove_snapshot_axes(
        &mut self, axes: &[Option<usize>], equal_axes: &mut Vec<(usize, usize)>, work: &mut usize,
    ) -> Option<()> {
        fn visible_eq(
            equal: &[(usize, usize)], axes: &[Option<usize>], work: &mut usize,
        ) -> Option<Vec<(usize, usize)>> {
            charge_call_work(work, equal.len().checked_mul(3)?)?;
            let mut promoted = HashMap::new();
            let mut output = Vec::new();
            for &(alias, root) in equal {
                let Some(alias) = axes[alias] else { continue; };
                if let Some(root) = axes[root] { output.push((alias, root)); }
                else if let Some(&root) = promoted.get(&root) { output.push((alias, root)); }
                else { promoted.insert(root, alias); }
            }
            Some(output)
        }
        *equal_axes = visible_eq(equal_axes, axes, work)?;
        for row in &mut self.0 {
            charge_call_work(work, axes.len().checked_add(1)?)?;
            let eq = visible_eq(row.equal_axes(), axes, work)?;
            if let Some(affine) = &row.affine {
                let count = affine.members.len().checked_add(1)?;
                charge_call_work(work, count.checked_mul(row.equal_axes().len().checked_add(count)?)?)?;
                let visible = |old: usize| axes[old].or_else(|| {
                    row.equal_axes().iter().find_map(|&(alias, root)| (root == old).then_some(axes[alias]).flatten())
                });
                let mut endpoints: Vec<_> = std::iter::once((affine.anchor, None))
                    .chain(affine.members.iter().map(|(axis, transform)| (*axis, Some(transform))))
                    .filter_map(|(old, transform)| visible(old).map(|new| (new, transform)))
                    .collect();
                endpoints.sort_unstable_by_key(|&(new, _)| new);
                row.affine = match endpoints.split_first() {
                    Some((&(anchor, anchor_transform), others)) if !others.is_empty() => {
                        let inverse = anchor_transform.map(|transform| Rc::new(transform.inverse()));
                        let members = others.iter().map(|&(axis, transform)| {
                            let transform = match (&inverse, transform) {
                                (None, Some(transform)) => Rc::clone(transform),
                                (Some(inverse), None) => Rc::clone(inverse),
                                (Some(inverse), Some(transform)) => Rc::new(transform.compose(inverse)),
                                (None, None) => unreachable!("affine class has one identity anchor"),
                            };
                            (axis, transform)
                        }).collect();
                        Some(Rc::new(IntegerAffineAxes { anchor, members }))
                    }
                    _ => None,
                };
            }
            if let Some(ProjectedEntry::Linked(old)) = row.entry.as_deref() {
                row.entry = Some(Rc::new(match axes[*old] {
                    Some(axis) => ProjectedEntry::Linked(axis),
                    None => ProjectedEntry::Detached(Rc::clone(&row.cells[*old])),
                }));
            }
            let mut index = 0;
            row.cells.retain(|_| { let keep = axes[index].is_some(); index += 1; keep });
            row.equal_axes = (!eq.is_empty()).then(|| Rc::new(eq));
        }
        Some(())
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
            CheckerPredicate::NamedBoolEntry { .. } | CheckerPredicate::EntryDomain { .. }
            | CheckerPredicate::DirectiveDisjunction { .. } | CheckerPredicate::ProofReflexiveBranch(_) => return None,
            CheckerPredicate::Integer(predicate) => {
                let index = self.axes.iter().position(|binding| **binding == predicate.binding)?;
                self.state.split_equal_axis(index, &predicate.operator, &predicate.bound)?
            }
        };
        Some(
            shows_return_equality(&yes, &self.axes, &self.value, then_claim)?
                && shows_return_equality(&no, &self.axes, &self.value, else_claim)?,
        )
    }

}

#[cfg(feature = "internal-checker")]
pub(super) struct ContextualUnitRefutation {
    pub(super) entry: BigInt,
    pub(super) exit: BigInt,
    pub(super) explicit: Option<BigInt>,
}

/// Checks one arbitrary typed contextual Unit root in the ordinary symbolic
/// row evaluator. Hidden copies preserve the original explicit argument
/// and contextual entry even if the body subsequently changes either place.
#[cfg(feature = "internal-checker")]
pub(super) fn check_symbolic_contextual_unit(
    calls: &FunctionCallSites, amount: Option<&BigInt>, add: bool,
    sink: &mut dyn for<'row> FnMut(IntegerCheckFailure<'row>),
) -> Option<(bool, Option<ContextualUnitRefutation>)> {
    let (formal, ty) = calls.contextual_formal()?;
    if !matches!(calls.boundary.results.as_slice(), [out] if out.ty == ScalarType::Unit)
        || !calls.call_occurrences.is_empty() || !calls.allocation_length_calls.is_empty()
    {
        return None;
    }
    let explicit = match calls.boundary.parameters.as_slice() {
        [] => None,
        [parameter] if parameter.ordinal == 0 && is_integer_type(&parameter.ty) => {
            let [binding] = calls.boundary.checker_parameter_bindings.as_deref()? else { return None; };
            Some((binding, &parameter.ty, ReferenceBindingId {
                kind: super::ReferenceBindingKind::CheckerSnapshotField(1), ..binding.clone()
            }))
        }
        _ => return None,
    };
    if amount.is_none() && !explicit.as_ref().is_some_and(|(_, argument_ty, _)| *argument_ty == ty) {
        return None;
    }
    let snapshots = ReferenceBindingId {
        kind: super::ReferenceBindingKind::CheckerSnapshotField(0), ..formal.clone()
    };
    let mut roots = IntegerRoots {
        allow_ordinary_contextual_places: true,
        allow_extended_scalar_locals: true,
        symbolic_context_work: Some(0),
        symbolic_context_entry_amount: amount.is_none(),
        ..IntegerRoots::default()
    };
    let mut rows = IntegerInputs(vec![IntegerRow::new(vec![Rc::new(IntegerSet::from_type(ty)?)])]);
    roots.add(formal, ty);
    rows.append_ordinary_copy(0, &mut roots)?;
    roots.add(&snapshots, ty);
    if let Some((binding, ty, snapshot)) = &explicit {
        let [row] = rows.0.as_mut_slice() else { return None; };
        charge_call_work(roots.symbolic_context_work.as_mut()?, row.equal_axes().len().checked_add(4)?)?;
        let domain = Rc::new(IntegerSet::from_type(ty)?);
        row.push(Rc::clone(&domain));
        roots.add(binding, ty);
        row.append_alias(2, domain);
        roots.add(snapshot, ty);
    }
    let mut checked = IntegerFailureCollection {
        continuation: IntegerRequirementContinuation::Empty,
        projected: None, allow_projected_calls: false, allow_contextual_local_transfers: false,
        sink: &mut |failure| sink(failure),
    };
    let mut imports: &mut [Option<ImportedLocal<'_>>] = &mut [];
    let IntegerControl::Continue(rows) = walk_integer_block::<false, true, _>(
        calls, &mut roots, calls.structure.root, rows, None, None, &mut None, &mut Some(&mut checked),
        &mut imports, (),
    )? else { return None; };
    let active = roots.reached_obligation || !rows.is_unreachable();
    let work = roots.symbolic_context_work.as_mut()?;
    let (type_min, type_max) = super::typecheck::integer_bounds(ty)?;
    let mut first_refutation = None;
    let width = if explicit.is_some() { 4 } else { 2 };
    for row in rows.0 {
        charge_call_work(work, 3 + 2 * width)?;
        // This profile retains only equality copies and independent cells;
        // refusing unmodeled relations avoids comparing loose marginals.
        if row.len() != width || row.affine.is_some() || row.entry.is_some() { return None; }
        // A typed exact amount on this row is the same checked
        // translation as a written literal. Keep the original contextual
        // entry and exit relation rather than projecting either axis.
        let amount = match amount {
            Some(literal) => literal,
            None => row.get(3)?.singleton_value()?,
        };
        let expected_at = |entry: &BigInt| if add { entry + amount } else { entry - amount };
        let (old_min, old_max) = row.get(1)?.extremes()?;
        // Translation is monotone, even for a disconnected row. Check both
        // extrema of every surviving entry domain before exposing a result.
        let expected_min = expected_at(old_min);
        let expected_max = expected_at(old_max);
        if expected_min < type_min || expected_max > type_max { return None; }
        if first_refutation.is_some() { continue; }
        let (new_min, new_max) = row.first()?.extremes()?;
        let counterexample = |entry: &BigInt, exit: &BigInt| -> Option<ContextualUnitRefutation> {
            let explicit = if explicit.is_some() {
                // The witness must extend this same row and respect equal
                // coordinates. Only independent extra axes choose their min.
                let slot = row.canonical_axis(3);
                let value = if slot == row.canonical_axis(1) { entry }
                    else if slot == row.canonical_axis(0) { exit }
                    else { row.get(3)?.extremes()?.0 };
                Some(value.clone())
            } else { None };
            Some(ContextualUnitRefutation { entry: entry.clone(), exit: exit.clone(), explicit })
        };
        if row.canonical_axis(0) == row.canonical_axis(1) {
            if &expected_min != old_min {
                first_refutation = Some(counterexample(old_min, old_min)?);
            }
        } else {
            for (entry, exit, expected) in [
                (old_min, new_min, &expected_min), (old_min, new_max, &expected_min), (old_max, new_min, &expected_max),
            ] {
                if expected != exit {
                    first_refutation = Some(counterexample(entry, exit)?);
                    break;
                }
            }
        }
    }
    Some((active, first_refutation))
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
    #[cfg(feature = "internal-checker")]
    UnitHistory(CallableHistoryEntry),
}

#[cfg(feature = "internal-checker")]
pub(super) struct CallableHistoryEntry(IntegerRow);

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
    allow_extended_scalar_locals: bool,
) -> Option<(bool, Vec<IntegerCheckFailure<'a>>)> {
    let mut failures = Vec::new();
    let mut sink = |failure| failures.push(failure);
    let (active, _) = if allow_ordinary_integer_locals && (calls.structure.has_ordinary_integer_operations()
        || ordinary_tail_type(calls, true) == Some(&ScalarType::Bool)) {
        collect_integer_requirement_failures_in_scope(
            calls, IntegerRequirementEntry::Arbitrary, None, false, false, &mut sink, OrderedRootConditions::<true, true>(body, allow_extended_scalar_locals),
        )
    } else {
        collect_integer_requirement_failures_in_scope(
            calls, IntegerRequirementEntry::Arbitrary, None, false, false, &mut sink, OrderedRootConditions::<true>(body, false),
        )
    }?;
    Some((active, failures))
}

#[cfg(feature = "internal-checker")]
pub(super) enum CallableHistoryCompletion {
    Exhausted(bool),
    LimitReached,
}

#[cfg(feature = "internal-checker")]
pub(super) struct UnitHistoryPiece {
    domains: Vec<Rc<IntegerSet>>,
    steps: Vec<CallableHistoryStep>,
}

#[cfg(feature = "internal-checker")]
pub(super) enum CallableHistoryCall {
    UnitPartitions(Vec<UnitHistoryPiece>),
    ScalarPartitions(Vec<IntegerReturnPartition>, Vec<Vec<CallableHistoryStep>>),
    ScalarPairPartitions(ProjectedIntegerPair, Vec<Vec<CallableHistoryStep>>),
    LimitReached,
}

#[cfg(feature = "internal-checker")]
pub(super) type CallableHistoryCallHandler<'run> = dyn FnMut(
    &FunctionCallSites, OccurrenceOrdinal, &CheckedCallSignature, NestedIntegerArgument<'_>,
    Vec<CallableHistoryStep>,
) -> Option<CallableHistoryCall> + 'run;

#[cfg(feature = "internal-checker")]
fn accept_callable_history_return(
    histories: &std::cell::RefCell<CallableHistories<'_>>, result: CallableHistoryCall, expected: &ScalarType,
) -> Option<ActualCallResult> {
    match result {
        CallableHistoryCall::UnitPartitions(pieces) if *expected == ScalarType::Unit => {
            let mut trace = histories.borrow_mut();
            if trace.received_unit_paths.is_some() { return None; }
            trace.received_unit_paths = Some(pieces);
            Some(ActualCallResult::Unit)
        }
        CallableHistoryCall::ScalarPartitions(pieces, steps) if is_integer_type(expected) && pieces.len() == steps.len() => {
            let mut trace = histories.borrow_mut();
            if trace.scalar_return_steps.is_some() { return None; }
            trace.scalar_return_steps = Some(steps);
            Some(ActualCallResult::PartitionedInteger(pieces))
        }
        CallableHistoryCall::ScalarPairPartitions(pair, steps) if is_integer_type(expected) && pair.pieces.len() == steps.len() => {
            let mut trace = histories.borrow_mut();
            if trace.scalar_return_steps.is_some() { return None; }
            trace.scalar_return_steps = Some(steps);
            Some(ActualCallResult::PartitionedIntegerPair(pair))
        }
        CallableHistoryCall::LimitReached => {
            histories.borrow_mut().stopped = true;
            None
        }
        _ => None,
    }
}

#[cfg(all(test, feature = "internal-checker"))]
pub(super) fn search_closed_callable_histories(
    calls: &FunctionCallSites, entry: IntegerRequirementEntry<'_>, work: &std::cell::Cell<usize>,
    nested: Option<&mut CallableHistoryCallHandler<'_>>,
    select: impl FnMut(IntegerCheckFailure<'_>, Vec<CallableHistoryStep>) -> Option<bool>,
) -> Option<CallableHistoryCompletion> {
    search_callable_history_frame(calls, entry, work, None, false, HistoryFrameEffects::None, nested, select).map(|(end, _)| end)
}

#[cfg(feature = "internal-checker")]
pub(super) fn search_callable_history_frame(
    calls: &FunctionCallSites, entry: IntegerRequirementEntry<'_>, work: &std::cell::Cell<usize>,
    step_owner: Option<usize>, capture_unit: bool, frame_effects: HistoryFrameEffects, mut nested: Option<&mut CallableHistoryCallHandler<'_>>,
    mut select: impl FnMut(IntegerCheckFailure<'_>, Vec<CallableHistoryStep>) -> Option<bool>,
) -> Option<(CallableHistoryCompletion, Vec<UnitHistoryPiece>)> {
    if capture_unit && (!calls.checker_returns_unit() || !(1..=2).contains(&calls.boundary.parameters.len())) { return None; }
    let histories = std::cell::RefCell::new(CallableHistories {
        work, outgoing_call: None, scalar_return_steps: None, step_owner, stopped: false, nodes: Vec::new(), interned: HashMap::new(),
        unit_return_arity: capture_unit.then_some(calls.boundary.parameters.len()), unit_return_paths: None, received_unit_paths: None,
    });
    let mut unsupported = false;
    let mut sink = |mut failure: IntegerCheckFailure<'_>| {
        let mut trace = histories.borrow_mut();
        let selected = (|| {
            let history = failure.state.0.first()?.callable_history;
            trace.charge(failure.state.0.len().checked_add(failure.axes.len())?)?;
            let steps = trace.steps(history)?;
            // The driver submits one complete structural packet.
            for row in &mut failure.state.0 {
                if row.callable_history != history { return None; }
                row.callable_history = 0;
            }
            select(failure, steps)
        })();
        match selected {
            Some(true) => trace.stopped = true,
            Some(false) => {}
            None => { unsupported = true; trace.stopped = true; }
        }
    };
    let allow_calls = nested.is_some();
    let mut call = |caller: &FunctionCallSites, site, signature: &CheckedCallSignature,
                    argument: NestedIntegerArgument<'_>, expected: &ScalarType| {
        let history = histories.borrow_mut().outgoing_call.take()?;
        let returned = nested.as_deref_mut()?(caller, site, signature, argument, history)?;
        accept_callable_history_return(&histories, returned, expected)
    };
    let result = collect_integer_requirement_failures_in_scope(calls, entry,
        if allow_calls { Some(&mut call) } else { None }, allow_calls, false, &mut sink, TrackCallableHistory(&histories, frame_effects));
    if unsupported { return None; }
    if histories.borrow().stopped { return Some((CallableHistoryCompletion::LimitReached, Vec::new())); }
    let (active, _) = result?;
    let pieces = if capture_unit {
        histories.borrow_mut().unit_return_paths.take()?
    } else { Vec::new() };
    Some((CallableHistoryCompletion::Exhausted(active), pieces))
}

#[cfg(all(test, feature = "internal-checker"))]
pub(super) fn test_first_complete_history_packet(
    calls: &FunctionCallSites, mut inspect: impl FnMut(IntegerCheckFailure<'_>),
) -> Option<()> {
    // Two separately represented incoming exact rows, both at the genuine
    // start of the same prepared body. No branch labels or histories invented.
    let [parameter] = calls.boundary.parameters.as_slice() else { return None; };
    let [binding] = calls.boundary.checker_parameter_bindings.as_deref()? else { return None; };
    if parameter.ty != ScalarType::U8 { return None; }
    let mut roots = IntegerRoots { parameters: 1, ..IntegerRoots::default() };
    roots.add(binding, &parameter.ty);
    let work = std::cell::Cell::new(0);
    let tracker = std::cell::RefCell::new(CallableHistories {
        work: &work, outgoing_call: None, scalar_return_steps: None, step_owner: None,
        stopped: false, nodes: Vec::new(), interned: HashMap::new(),
        unit_return_arity: None, unit_return_paths: None, received_unit_paths: None,
    });
    roots.callable_history = Some(&tracker);
    let mut selected = false;
    let mut sink = |failure| {
        assert!(!selected, "cap 1");
        selected = true;
        inspect(failure);
        tracker.borrow_mut().stopped = true;
    };
    let mut collection = Some(IntegerFailureCollection {
        continuation: IntegerRequirementContinuation::Empty, projected: None,
        allow_projected_calls: false, allow_contextual_local_transfers: false, sink: &mut sink,
    });
    let inputs = IntegerInputs([BigInt::from(0u8), BigInt::from(7u8)].iter()
        .map(|value| exact_integer_entry_row(&[value])).collect());
    let mut failures = collection.as_mut();
    let mut imports: &mut [Option<ImportedLocal<'_>>] = &mut [];
    let _ = walk_integer_block::<false, true, _>(calls, &mut roots, calls.structure.root, inputs, None,
        None, &mut None, &mut failures, &mut imports, TrackCallableHistory(&tracker, HistoryFrameEffects::None));
    selected.then_some(())
}

/// Preserve the actual child's selected branch history alongside each
/// original typed return partition. Each pure frame walks only once per
/// exact input row; extraction reads only the already retained return rows.
#[cfg(feature = "internal-checker")]
pub(super) fn trace_closed_projected_scalar_return(
    calls: &FunctionCallSites, entry: CallableHistoryEntry, expected: &ScalarType, step_owner: Option<usize>, work: &std::cell::Cell<usize>,
    mut nested: Option<&mut CallableHistoryCallHandler<'_>>,
) -> Option<CallableHistoryCall> {
    let histories = std::cell::RefCell::new(CallableHistories {
        work, outgoing_call: None, scalar_return_steps: None, step_owner, stopped: false, nodes: Vec::new(), interned: HashMap::new(),
        unit_return_arity: None, unit_return_paths: None, received_unit_paths: None,
    });
    histories.borrow().charge(calls.occurrences.len())?;
    let CallableHistoryEntry(mut row) = entry;
    let inputs = row.cells.clone();
    let mut cursor = match inputs.as_slice() {
        [domain] => ExactCallCursor::projected(calls, Rc::clone(domain), expected)?,
        [_, _] => {
            // The only input row is the authenticated original call's
            // independent rectangle. Pair receipts retain its local anchor.
            row.projected_from = 1;
            ExactCallCursor::with_rows(calls, expected, IntegerInputs(vec![row]), None, Some([0, 1]))?
        }
        _ => return None,
    };
    cursor.roots.callable_history = Some(&histories);
    let allow_nested = nested.is_some();
    let mut call = |caller: &FunctionCallSites, site, signature: &CheckedCallSignature,
                    argument: NestedIntegerArgument<'_>, expected: &ScalarType| {
        let history = histories.borrow_mut().outgoing_call.take()?;
        let returned = nested.as_deref_mut()?(caller, site, signature, argument, history)?;
        accept_callable_history_return(&histories, returned, expected)
    };
    let ExactCallProgress::Done { active: false, result } =
        cursor.advance::<false>(calls, allow_nested.then_some(&mut call), false, false, false, false, &mut |_| {})?
    else { return None; };
    let mut receipt_work = work.get();
    let tracker = histories.borrow();
    let rows = || cursor.returned.iter().flat_map(|returned|
        returned.state.0.iter().map(move |row| (returned.span, row)));
    // This route only admits original return rows, not independently
    // computed or synthetic receipts.
    let result = match (result, inputs.as_slice()) {
        (ActualCallResult::PartitionedInteger(pieces), [entry]) => {
            if !unary_partitions_cover(&pieces, entry, &mut receipt_work)? { return None; }
            work.set(receipt_work);
            let steps = rows().zip(&pieces).map(|((span, row), piece)| {
                tracker.charge(1)?;
                if span != piece.span || !Rc::ptr_eq(row.entry_domain(0), &piece.input) { return None; }
                tracker.steps(row.callable_history)
            }).collect::<Option<Vec<_>>>()?;
            if steps.len() != pieces.len() { return None; }
            CallableHistoryCall::ScalarPartitions(pieces, steps)
        }
        (ActualCallResult::PartitionedIntegerPair(pair), [left, right]) => {
            if !covers_pair_inputs(&pair, [left, right], 1, &mut receipt_work)? { return None; }
            work.set(receipt_work);
            let steps = rows().zip(&pair.pieces).map(|((span, row), piece)| {
                tracker.charge(1)?;
                if span != piece.span || row.projected_from != piece.anchor
                    || !piece.inputs.iter().zip(&row.cells).all(|(input, cell)| Rc::ptr_eq(input, cell))
                { return None; }
                tracker.steps(row.callable_history)
            }).collect::<Option<Vec<_>>>()?;
            if steps.len() != pair.pieces.len() { return None; }
            CallableHistoryCall::ScalarPairPartitions(pair, steps)
        }
        _ => return None,
    };
    Some(result)
}

#[cfg(feature = "internal-checker")]
#[derive(Clone, Copy)]
struct RuntimeFixedIndexObserver<'a, 'proof> {
    conditions: OrderedRootConditions<'a, true, true>,
    calls: &'a FunctionCallSites,
    discovered: &'proof std::cell::RefCell<HashMap<OccurrenceOrdinal, (&'a super::reference::CheckerOrdinaryScalarLocal, usize)>>,
}

#[cfg(feature = "internal-checker")]
impl<'a> ScopedAuthoredReturn<'a> for RuntimeFixedIndexObserver<'a, '_> {
    const ENABLED: bool = false;
    const ALLOW_ORDINARY_INTEGER_LOCALS: bool = true;
    const OBSERVE_RUNTIME_FIXED_INDEX: bool = true;

    fn allows_extended_scalar_locals(self) -> bool { true }

    fn at(
        self, _: &FunctionCallSites, _: BlockOrdinal, _: FinalOutputSlot, _: &ScalarType,
        _: &IntegerRoots<'_>, _: &IntegerInputs,
    ) -> Option<&'a BigInt> { None }

    fn split_unprepared(
        self, calls: &FunctionCallSites, slot: RootBranchSlot, roots: &IntegerRoots<'_>, path: IntegerInputs,
    ) -> Option<(IntegerInputs, IntegerInputs, Option<usize>)> {
        self.conditions.split_unprepared(calls, slot, roots, path)
    }

    fn prefer_legacy_root_compound(
        self, calls: &FunctionCallSites, block: BlockOrdinal, condition: OccurrenceOrdinal, roots: &IntegerRoots<'_>,
    ) -> bool {
        self.conditions.prefer_legacy_root_compound(calls, block, condition, roots)
    }

    fn observe_runtime_fixed_index(
        self, origin: &'a super::reference::CheckerOrdinaryScalarLocal, extent: usize,
    ) -> Option<()> {
        let shape = self.calls.fixed_index_read_shape(origin.source)?;
        let super::reference::CheckerOrdinaryScalarLocalKind::SnapshotRead {
            index: super::reference::CheckerSnapshotReadIndex::ArrayPath(steps), ..
        } = &origin.kind else { return None; };
        if !matches!(steps.as_ref(), [step]
            if step.length == extent && shape.extent == extent
                && matches!(step.offset, super::reference::CheckerSnapshotArrayOffset::Lexical(index) if index == shape.index))
        {
            return None;
        }
        if let Some((previous, length)) = self.discovered.borrow_mut().insert(origin.source, (origin, extent)) {
            if !std::ptr::eq(previous, origin) || length != extent { return None; }
        }
        Some(())
    }
}

/// In-run coverage for the exact immutable prepared function and access.
/// This cannot be serialized or rejoined by span. It is not a borrow result
/// or emission instruction. Until Build shares these prepared identities its
/// normal checked-access edge is retained.
#[cfg(feature = "internal-checker")]
#[allow(dead_code)]
pub(super) struct RuntimeFixedIndexEvidence<'a> {
    body: &'a FunctionCallSites,
    accesses: HashMap<OccurrenceOrdinal, (&'a super::reference::CheckerOrdinaryScalarLocal, usize)>,
}

#[cfg(feature = "internal-checker")]
#[allow(dead_code)]
impl RuntimeFixedIndexEvidence<'_> {
    pub(super) fn covers_read(&self, read: OccurrenceOrdinal, extent: usize) -> bool {
        self.accesses.get(&read).is_some_and(|&(_, length)| length == extent)
    }

    pub(super) fn authorizes(
        &self, calls: &FunctionCallSites, access: &super::reference::CheckerOrdinaryScalarLocal, extent: usize,
    ) -> bool {
        std::ptr::eq(calls, self.body) && self.accesses.get(&access.source).is_some_and(|&(seen, length)|
            std::ptr::eq(seen, access) && length == extent)
    }
}

#[cfg(feature = "internal-checker")]
pub(super) fn observe_runtime_fixed_indices<'a>(
    calls: &'a FunctionCallSites, body: &'a super::ScalarBlock,
) -> Option<RuntimeFixedIndexEvidence<'a>> {
    if body.unsafe_context || !calls.checker_fixed_index_analysis_frame() || !calls.call_occurrences.is_empty()
        || !calls.structure.checker_direct_targets.is_empty() || !calls.callable_escapes.is_empty()
        || !calls.structure.module_storage_uses.is_empty() || calls.contextual_formal().is_some()
        || !calls.checker_returns_unit()
    {
        return None;
    }
    let discovered = std::cell::RefCell::new(HashMap::new());
    let observer = RuntimeFixedIndexObserver { conditions: OrderedRootConditions(body, true), calls, discovered: &discovered };
    // Unrestricted typed arguments, no proof entry filter, no imported
    // summaries, erased requirements, captured state or effects.
    let mut failed = false;
    let (active, continuation) = collect_integer_requirement_failures_in_scope(
        calls, IntegerRequirementEntry::Arbitrary, None, false, false, &mut |_| failed = true, observer,
    )?;
    if failed || active || !matches!(continuation, IntegerRequirementContinuation::Feasible) { return None; }
    let accesses = discovered.into_inner();
    (!accesses.is_empty()).then_some(RuntimeFixedIndexEvidence { body: calls, accesses })
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
    let scope = OrderedRootEntryEquality { conditions: OrderedRootConditions(body, false), old, guard, observed: &observed };
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
    collect_ordered_root_literal_proof_with_imports::<TOTAL>(
        calls, body, OrderedReturnedGoal::Literal(relation, bound), predicate, entry_filter, &mut [], false,
    )
}

#[cfg(feature = "internal-checker")]
pub(super) fn collect_imported_root_literal_proof<'a>(
    calls: &'a FunctionCallSites, body: &'a super::ScalarBlock, relation: IntegerRelation, bound: &BigInt,
    entry_filter: &[CheckerPredicate], imports: &mut [Option<ImportedLocal<'a>>],
) -> Option<(bool, Vec<IntegerCheckFailure<'a>>, bool, Option<SymbolicLiteralFailure<'a>>)> {
    collect_ordered_root_literal_proof_with_imports::<true>(
        calls, body, OrderedReturnedGoal::Literal(relation, bound), None, Some(entry_filter), imports, false,
    )
}

#[cfg(feature = "internal-checker")]
pub(super) fn collect_imported_root_entry_relationship<'a>(
    calls: &'a FunctionCallSites, body: &'a super::ScalarBlock, expected: &IntegerAffineTransform,
    entry_filter: &[CheckerPredicate], imports: &mut [Option<ImportedLocal<'a>>],
    qualified_conditional_return: bool,
) -> Option<(bool, Vec<IntegerCheckFailure<'a>>, bool, Option<SymbolicLiteralFailure<'a>>)> {
    let [input] = calls.boundary.parameters.as_slice() else { return None; };
    if imports.is_empty() || &input.ty != integer_tail_type(calls)? { return None; }
    collect_ordered_root_literal_proof_with_imports::<true>(
        calls, body, OrderedReturnedGoal::Entry(expected), None, Some(entry_filter), imports, qualified_conditional_return,
    )
}

#[cfg(feature = "internal-checker")]
pub(super) fn collect_ordered_root_quantified_proof<'a>(
    calls: &'a FunctionCallSites, body: &'a super::ScalarBlock, goal: QuantifiedIntegerReturn<'_>,
    entry_filter: &[CheckerPredicate],
) -> Option<(bool, Vec<IntegerCheckFailure<'a>>, bool, Option<SymbolicLiteralFailure<'a>>)> {
    collect_ordered_root_literal_proof_with_imports::<true>(
        calls, body, OrderedReturnedGoal::Quantified(goal), None, Some(entry_filter), &mut [], false,
    )
}

#[cfg(feature = "internal-checker")]
fn collect_ordered_root_literal_proof_with_imports<'a, const TOTAL: bool>(
    calls: &'a FunctionCallSites, body: &'a super::ScalarBlock, goal: OrderedReturnedGoal<'_>,
    predicate: Option<&IntegerPredicate>, entry_filter: Option<&[CheckerPredicate]>,
    imports: &mut [Option<ImportedLocal<'a>>], qualified_conditional_return: bool,
) -> Option<(bool, Vec<IntegerCheckFailure<'a>>, bool, Option<SymbolicLiteralFailure<'a>>)> {
    let bounds;
    let guard = if let Some(predicate) = predicate {
        let OrderedReturnedGoal::Literal(relation, bound) = goal else { return None; };
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
    let (imported_binding, imported_relation_work) = if imports.is_empty() {
        (None, matches!(goal, OrderedReturnedGoal::Quantified(_)).then(|| std::cell::RefCell::new(0)))
    } else {
        if imports.len() > 4 || imports.iter().any(Option::is_none) { return None; }
        let last = imports.last()?.as_ref()?;
        let relational = matches!(last.values, ImportedIntegerValues::Shared {
            result: ImportedVerifiedReturn::EntryAlias(_) | ImportedVerifiedReturn::EntryAffine { .. }, ..
        });
        let binding = &calls.structure.checker_integer_call_locals.get(&(calls.structure.root, last.position))?.binding;
        (Some(binding), (relational || matches!(goal, OrderedReturnedGoal::Entry(_))).then(|| std::cell::RefCell::new(0)))
    };
    let observed = std::cell::RefCell::new((false, None));
    let mut failures = Vec::new();
    let mut sink = |failure| failures.push(failure);
    let scope = OrderedRootLiteralProof::<TOTAL> {
        conditions: OrderedRootConditions(body, false), goal, guard: guard.as_ref(), imported_binding, qualified_conditional_return,
        imported_relation_work: imported_relation_work.as_ref(), observed: &observed,
    };
    let active = if TOTAL {
        let mut collected = IntegerFailureCollection {
            continuation: IntegerRequirementContinuation::Empty, projected: None, allow_projected_calls: false, allow_contextual_local_transfers: false, sink: &mut sink,
        };
        let coverage = if imports.is_empty() {
            run_integer_callable::<false, true, _>(
                calls, integer_tail_type(calls), entry_filter, None, &mut [], None, None, Some(&mut collected), false, scope,
            )
        } else {
            run_integer_callable::<true, true, _>(
                calls, integer_tail_type(calls), entry_filter, None, imports, None, None, Some(&mut collected), false, scope,
            )
        };
        match coverage?.ok()? {
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
    let observed = std::cell::RefCell::new(BatchedLiteralEvidence { returned: false, work: 0, pending: checks.len(), checks });
    let mut failures = Vec::new();
    let mut sink = |failure| failures.push(failure);
    let (active, _) = collect_integer_requirement_failures_in_scope(
        calls, IntegerRequirementEntry::Arbitrary, None, false, false, &mut sink,
        OrderedRootLiteralProofs::<STEP_FIRST, false> { conditions: OrderedRootConditions(body, false), observed: &observed },
    )?;
    Some((active, failures, observed.into_inner().returned))
}

#[cfg(feature = "internal-checker")]
pub(super) fn collect_pure_affine_literal_proofs<'a>(
    calls: &'a FunctionCallSites, body: &'a super::ScalarBlock,
    filter: &[CheckerPredicate], checks: &mut [SymbolicLiteralCheck<'_, 'a>],
) -> Option<bool> {
    collect_closed_literal_proofs(calls, body, integer_tail_type(calls)?, filter, checks)
}

#[cfg(feature = "internal-checker")]
pub(super) fn collect_closed_literal_proofs<'a>(
    calls: &'a FunctionCallSites, body: &'a super::ScalarBlock, expected: &ScalarType,
    filter: &[CheckerPredicate], checks: &mut [SymbolicLiteralCheck<'_, 'a>],
) -> Option<bool> {
    let tail = closed_literal_tail_type(calls, expected)?;
    let observed = std::cell::RefCell::new(BatchedLiteralEvidence { returned: false, work: 0, pending: checks.len(), checks });
    let mut sink = |_| {};
    let mut collected = IntegerFailureCollection {
        continuation: IntegerRequirementContinuation::Empty, projected: None, allow_projected_calls: false,
        allow_contextual_local_transfers: false, sink: &mut sink,
    };
    // This caller admitted a pure producer before entering the shared
    // filter, so evaluation cannot hide an obligation outside the filter.
    run_integer_callable::<false, true, _>(
        calls, Some(tail), Some(filter), None, &mut [], None, None, Some(&mut collected), false,
        OrderedRootLiteralProofs::<false, true> { conditions: OrderedRootConditions(body, false), observed: &observed },
    )?.ok()?;
    Some(observed.into_inner().returned)
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
        OrderedRootMultipleProof { conditions: OrderedRootConditions(body, false), observed: &observed },
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
        #[cfg(feature = "internal-checker")]
        IntegerRequirementEntry::UnitHistory(CallableHistoryEntry(row)) => {
            // No projected continuation is exported from the generic driver.
            // The history tracker retains each full returning rectangle.
            if scope.callable_history().is_none() || !calls.checker_returns_unit() || row.iter().any(|domain| domain.is_empty()) {
                return None;
            }
            (None, None, Some((0, row)))
        }
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
    exact_call_argument_in(structure, call, slot, allow_bool, false, lexical)
}

#[cfg(feature = "internal-checker")]
pub(super) fn exact_named_character_argument_at<'a>(
    structure: &'a OccurrenceStructure, call: OccurrenceOrdinal,
    lexical: impl FnOnce(OccurrenceOrdinal, &ScalarType) -> Option<&'a BigInt>,
) -> Option<ExactIntegerArgument<'a>> {
    exact_call_argument_in(structure, call, 0, true, true, lexical)
}

fn exact_call_argument_in<'a>(
    structure: &'a OccurrenceStructure, call: OccurrenceOrdinal, slot: usize, allow_bool: bool, allow_char: bool,
    lexical: impl FnOnce(OccurrenceOrdinal, &ScalarType) -> Option<&'a BigInt>,
) -> Option<ExactIntegerArgument<'a>> {
    let (argument, ty) = checked_scalar_call_argument_at(structure, call, slot, allow_bool, allow_char)?;
    let value = scalar_call_literal(structure, argument, ty, allow_bool).or_else(|| lexical(argument, ty))?;
    Some((ty, value))
}

fn checked_scalar_call_argument_at(
    structure: &OccurrenceStructure, call: OccurrenceOrdinal, slot: usize, allow_bool: bool, allow_char: bool,
) -> Option<(OccurrenceOrdinal, &ScalarType)> {
    let OccurrenceNode::Expression(argument) = structure.edges.get(&(
        OccurrenceParent::Expression(call), OccurrenceEdge::CallArgument(slot),
    ))? else {
        return None;
    };
    let ty = structure.checked_types.expressions.get(argument)?;
    (is_integer_type(ty) || (allow_bool && *ty == ScalarType::Bool)
        || (allow_char && *ty == ScalarType::Char)).then_some((*argument, ty))
}

fn scalar_call_literal<'a>(structure: &'a OccurrenceStructure, argument: OccurrenceOrdinal, ty: &ScalarType, ordinary: bool)
    -> Option<&'a BigInt>
{
    #[cfg(feature = "internal-checker")]
    if matches!(*ty, ScalarType::Bool | ScalarType::Char) {
        return structure.checker_ordinary_literal_atoms.get(&argument);
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

#[cfg(feature = "internal-checker")]
pub(super) fn named_exact_mixed_pair<T>(parameters: &[T], ty: impl Fn(&T) -> &ScalarType) -> bool {
    matches!(parameters, [first, second] if matches!((ty(first), ty(second)),
        (ScalarType::Bool, integer) | (integer, ScalarType::Bool) if is_integer_type(integer)))
}

#[cfg(feature = "internal-checker")]
pub(super) fn named_stack_parameters(parameters: &[ScalarType]) -> bool {
    (matches!(parameters, [_] | [_, _]) && parameters.iter().all(is_integer_type))
        || named_exact_mixed_pair(parameters, |ty| ty)
}

#[cfg(feature = "internal-checker")]
pub(super) fn named_nested_scalar_parameters(parameters: &[ScalarType], result: &ScalarType) -> bool {
    if *result == ScalarType::Unit {
        return matches!(parameters, [] | [_] | [_, _]) && parameters.iter().all(is_integer_type);
    }
    if *result == ScalarType::Bool && matches!(parameters, [] | [ScalarType::Bool]) { return true; }
    (is_integer_type(result) || *result == ScalarType::Bool) && named_stack_parameters(parameters)
}

pub(super) enum ActualCallResult {
    Unit,
    EmptyUnit,
    #[cfg(feature = "internal-checker")]
    ContextualUnit { entry: BigInt, exit: BigInt },
    ProjectedUnit(Rc<IntegerSet>),
    ProjectedUnitPair(ProjectedUnitPair),
    Integer(BigInt),
    #[cfg(feature = "internal-checker")]
    FixedArray(Vec<BigInt>),
    #[cfg(feature = "internal-checker")]
    ResultPair([BigInt; 2]),
    #[cfg(feature = "internal-checker")]
    EmptyScalar,
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
    #[cfg(feature = "internal-checker")]
    span: super::ByteSpan,
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

pub(super) use super::analysis_budget::{charge_call_work, MAX_ACTUAL_CALL_DEPTH, MAX_PARTITION_TRANSFER_WORK};

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
#[derive(Clone, Copy)]
pub(super) enum PartitionedProofGoal<'a> {
    Literal(IntegerRelation, &'a BigInt),
    EntryEquality(usize),
    EntryAffineEquality { slot: usize, transform: &'a Rc<IntegerAffineTransform> },
}

#[cfg(feature = "internal-checker")]
pub(super) fn covers_all_named_pair_inputs(
    returned: &ActualCallResult, full: &[IntegerSet; 2], work: &mut usize,
) -> Option<bool> {
    let ActualCallResult::PartitionedIntegerPair(pair) = returned else { return None; };
    if pair.caller_axes != [0, 1] { return None; }
    covers_pair_inputs(pair, [&full[0], &full[1]], 0, work)
}

#[cfg(feature = "internal-checker")]
fn covers_pair_inputs(pair: &ProjectedIntegerPair, full: [&IntegerSet; 2], anchor: u16, work: &mut usize) -> Option<bool> {
    if pair.caller_axes != [0, 1] || full.iter().any(|axis| axis.is_empty()) { return None; }
    let [first_count, second_count] = full.map(IntegerSet::interval_count);
    charge_call_work(work, pair.pieces.len())?;
    let mut cost = [0usize; 2];
    for piece in &pair.pieces {
        if piece.anchor != anchor { return None; }
        for (total, axis) in cost.iter_mut().zip(&piece.inputs) {
            *total = total.checked_add(axis.interval_count())?;
        }
    }
    let sort_work = |count: usize| {
        let depth = (usize::BITS - count.leading_zeros()) as usize;
        count.checked_mul(depth.checked_mul(2)?.checked_add(4)?)
    };
    let scoped = anchor != 0;
    let base_cuts = if scoped { first_count.checked_mul(2)? } else { 2 };
    let cuts = cost[0].checked_mul(2)?.checked_add(base_cuts)?;
    charge_call_work(work, sort_work(cuts)?.checked_add(cost[0])?.checked_add(cost[1])?)?;
    let mut starts = Vec::with_capacity(cuts);
    if scoped {
        charge_call_work(work, pair.pieces.len().checked_mul(first_count.checked_add(second_count)?)?)?;
        for (lower, upper) in full[0].intervals() {
            starts.push(lower.clone());
            starts.push(upper + 1);
        }
    } else {
        let (minimum, maximum) = full[0].extremes()?;
        starts.push(minimum.clone());
        starts.push(maximum + 1);
    }
    for piece in &pair.pieces {
        if !piece.inputs[0].is_subset_of(full[0]) || !piece.inputs[1].is_subset_of(full[1]) { return None; }
        for (lower, upper) in piece.inputs[0].intervals() {
            starts.push(lower.clone());
            starts.push(upper + 1);
        }
    }
    starts.sort_unstable();
    starts.dedup();
    // Between consecutive first-axis boundaries, coverage by a rectangle
    // cannot change. Check its actual second-axis union in each slab.
    let mut active = Vec::new();
    for slab in starts.windows(2) {
        let first = &slab[0];
        if scoped {
            charge_call_work(work, 1usize.checked_add(first_count)?)?;
            if !full[0].contains(first) { continue; }
        }
        charge_call_work(work, pair.pieces.len().checked_add(cost[0])?.checked_add(sort_work(cost[1])?)?
            .checked_add(if scoped { second_count } else { 0 })?)?;
        active.clear();
        let last = &slab[1] - 1;
        for piece in &pair.pieces {
            if piece.inputs[0].covers_closed(first, &last) { active.push(piece.inputs[1].as_ref()); }
        }
        if !full[1].is_subset_of(&IntegerSet::union(active.iter().copied())) { return Some(false); }
    }
    Some(true)
}

#[cfg(feature = "internal-checker")]
pub(super) fn covers_all_named_mixed_pair_inputs(
    returned: &ActualCallResult, bool_axis: usize, full_integer: &IntegerSet, work: &mut usize,
) -> Option<bool> {
    let ActualCallResult::PartitionedIntegerPair(pair) = returned else { return None; };
    if pair.caller_axes != [0, 1] || bool_axis >= 2 { return None; }
    charge_call_work(work, pair.pieces.len().checked_mul(2)?)?;
    let numeric = 1 - bool_axis;
    let mut domains = [Vec::new(), Vec::new()];
    let mut intervals = [0usize; 2];
    for piece in &pair.pieces {
        if piece.anchor != 0 || matches!(piece.output, IntegerPairPartitionOutput::InputAffine(..)) { return None; }
        let lane = bool_result_lane(piece.inputs[bool_axis].singleton_value()?)?;
        let values = piece.inputs[numeric].as_ref();
        charge_call_work(work, values.interval_count().checked_add(full_integer.interval_count())?)?;
        if !values.is_subset_of(full_integer) { return None; }
        intervals[lane] = intervals[lane].checked_add(values.interval_count())?;
        domains[lane].push(values);
    }
    for (cells, count) in domains.into_iter().zip(intervals) {
        let depth = (usize::BITS - count.leading_zeros()) as usize;
        charge_call_work(work, count.checked_mul(depth.checked_mul(2)?.checked_add(4)?)?
            .checked_add(full_integer.interval_count())?)?;
        if !full_integer.is_subset_of(&IntegerSet::union(cells)) { return Some(false); }
    }
    Some(true)
}

#[cfg(all(test, feature = "internal-checker"))]
#[test]
fn named_mixed_pair_checks_complete_integer_domain_in_each_singleton_tag() {
    let zero = BigInt::from(0);
    let one = BigInt::from(1);
    let integer = IntegerSet::ordinary_bool_domain();
    let full = || IntegerSet::ordinary_bool_domain();
    let piece = |tag, values| IntegerPairReturnPartition {
        span: super::ByteSpan { start: 0, end: 0 }, anchor: 0,
        inputs: [Rc::new(tag), Rc::new(values)], output: IntegerPairPartitionOutput::InputAlias(1),
    };
    let coverage = |pieces| covers_all_named_mixed_pair_inputs(
        &ActualCallResult::PartitionedIntegerPair(ProjectedIntegerPair { caller_axes: [0, 1], pieces }), 0, &integer, &mut 0,
    );
    assert_eq!(coverage(vec![
        piece(IntegerSet::singleton(&zero), IntegerSet::singleton(&zero)),
        piece(IntegerSet::singleton(&one), IntegerSet::singleton(&one)),
    ]), Some(false));
    assert_eq!(coverage(vec![piece(full(), full())]), None, "merged numeric interval cannot certify two Bool lanes");
    assert_eq!(coverage(vec![piece(IntegerSet::singleton(&zero), full()), piece(IntegerSet::singleton(&one), full())]), Some(true));
}

#[cfg(all(test, feature = "internal-checker"))]
#[test]
fn named_pair_independence_allows_one_affine_output_but_not_coupled_inputs() {
    let values = Rc::new(IntegerSet::from_type(&ScalarType::I16).unwrap());
    let mut row = IntegerRow::new(vec![Rc::clone(&values), Rc::clone(&values), values]);
    let transform = Rc::new(IntegerAffineTransform { reflected: true, offset: BigInt::from(-1) });
    row.affine = Some(Rc::new(IntegerAffineAxes { anchor: 1, members: vec![(2, Rc::clone(&transform))] }));
    assert_eq!(row.require_independent_named_pair(&mut 0), Some(()));
    row.affine = Some(Rc::new(IntegerAffineAxes { anchor: 0, members: vec![(1, transform)] }));
    assert_eq!(row.require_independent_named_pair(&mut 0), None);
}

#[cfg(all(test, feature = "internal-checker"))]
#[test]
fn named_pair_coverage_does_not_confuse_marginals_with_rectangular_coverage() {
    let zero = BigInt::from(0);
    let one = BigInt::from(1);
    let full = [IntegerSet::ordinary_bool_domain(), IntegerSet::ordinary_bool_domain()];
    let piece = |x, y| IntegerPairReturnPartition {
        span: super::ByteSpan { start: 0, end: 0 }, anchor: 0,
        inputs: [Rc::new(IntegerSet::singleton(x)), Rc::new(IntegerSet::singleton(y))],
        output: IntegerPairPartitionOutput::InputAlias(1),
    };
    let coverage = |pieces| covers_all_named_pair_inputs(
        &ActualCallResult::PartitionedIntegerPair(ProjectedIntegerPair { caller_axes: [0, 1], pieces }),
        &full, &mut 0,
    );
    assert_eq!(coverage(vec![piece(&zero, &zero), piece(&one, &one)]), Some(false));
    assert_eq!(coverage(vec![piece(&zero, &zero), piece(&zero, &one), piece(&one, &zero), piece(&one, &one)]), Some(true));
}

#[cfg(feature = "internal-checker")]
pub(super) fn verify_named_pair_return<'a>(
    producer: &'a FunctionCallSites, returned: &ActualCallResult, guards: &[IntegerSet; 2],
    goal: PartitionedProofGoal<'_>, work: &mut usize,
) -> Option<PartitionedCallProof<'a>> {
    let ActualCallResult::PartitionedIntegerPair(pair) = returned else { return None; };
    if pair.caller_axes != [0, 1] { return None; }
    let [first, second] = producer.boundary.checker_parameter_bindings.as_deref()? else { return None; };
    let [left, right] = producer.boundary.parameters.as_slice() else { return None; };
    let [result] = producer.boundary.results.as_slice() else { return None; };
    if let PartitionedProofGoal::EntryEquality(slot) | PartitionedProofGoal::EntryAffineEquality { slot, .. } = goal {
        if producer.boundary.parameters.get(slot)?.ty != result.ty { return None; }
    }
    if let PartitionedProofGoal::EntryAffineEquality { slot, transform } = goal {
        if guards.iter().any(IntegerSet::is_empty) {
            return Some(PartitionedCallProof { applicable: false, covers_guard: false, failure: None });
        }
        let (min, max) = super::typecheck::integer_bounds(&result.ty)?;
        transform.fits(guards.get(slot)?, &min, &max)?;
    }
    let mut applicable = false;
    'pieces: for piece in &pair.pieces {
        if piece.anchor != 0 { return None; }
        if matches!(&piece.output, IntegerPairPartitionOutput::InputAlias(slot) | IntegerPairPartitionOutput::InputAffine(slot, _)
            if producer.boundary.parameters.get(*slot).is_none_or(|entry| entry.ty != result.ty))
        {
            return None;
        }
        let mut cells = [Rc::clone(&piece.inputs[0]), Rc::clone(&piece.inputs[1])];
        let cost = |axis: usize| cells[axis].interval_count().checked_add(guards[axis].interval_count());
        charge_call_work(work, 4usize.checked_add(cost(0)?)?.checked_add(cost(1)?)?)?;
        for (cell, guard) in cells.iter_mut().zip(guards) {
            match cell.intersection(guard) {
                IntegerIntersection::Changed(clipped) => *cell = Rc::new(clipped),
                IntegerIntersection::Empty => continue 'pieces,
                IntegerIntersection::Unchanged => continue,
            }
        }
        applicable = true;
        let constrain = |selected: &mut Rc<IntegerSet>, relation: IntegerRelation, bound: &BigInt, work: &mut usize| {
            charge_call_work(work, selected.interval_count().checked_mul(2)?.checked_add(1)?)?;
            let rejected = selected.keep_relation(relation, bound);
            let Some((first, _)) = rejected.extremes() else { return Some(None); };
            let example = first.clone();
            *selected = Rc::new(rejected);
            Some(Some(example))
        };
        let unequal = || IntegerRelation::from_operator(&BinaryOperator::NotEqual);
        let returned = match (goal, &piece.output) {
            (PartitionedProofGoal::Literal(relation, bound), IntegerPairPartitionOutput::Singleton(values)) => {
                let singleton = values.singleton_value()?;
                if relation.accepts(singleton.cmp(bound)) { continue; }
                singleton.clone()
            }
            (PartitionedProofGoal::Literal(relation, bound), IntegerPairPartitionOutput::InputAlias(slot)) => {
                let Some(example) = constrain(cells.get_mut(*slot)?, relation.complement(), bound, work)? else { continue; };
                example
            }
            (PartitionedProofGoal::Literal(relation, bound), IntegerPairPartitionOutput::InputAffine(slot, transform)) => {
                let (relation, cut) = transform.pull_back_cut(relation.complement(), bound);
                let Some(example) = constrain(cells.get_mut(*slot)?, relation, &cut, work)? else { continue; };
                transform.apply_point(&example)
            }
            (PartitionedProofGoal::EntryEquality(slot), IntegerPairPartitionOutput::Singleton(values)) => {
                let singleton = values.singleton_value()?;
                if constrain(cells.get_mut(slot)?, unequal()?, singleton, work)?.is_none() { continue; }
                singleton.clone()
            }
            (PartitionedProofGoal::EntryEquality(slot), IntegerPairPartitionOutput::InputAffine(output, transform))
                if slot == *output =>
            {
                if transform.identity_everywhere() { continue; }
                let source = cells.get_mut(slot)?;
                let example = if let Some(fixed) = transform.integral_reflection_fixed_point() {
                    let Some(example) = constrain(source, unequal()?, &fixed, work)? else { continue; };
                    example
                } else {
                    source.extremes()?.0.clone()
                };
                transform.apply_point(&example)
            }
            (PartitionedProofGoal::EntryEquality(slot), IntegerPairPartitionOutput::InputAlias(output)
                | IntegerPairPartitionOutput::InputAffine(output, _)) =>
            {
                if slot == *output { continue; }
                let evaluate = |input: &BigInt| match &piece.output {
                    IntegerPairPartitionOutput::InputAffine(_, transform) => transform.apply_point(input),
                    _ => input.clone(),
                };
                let (first, last) = cells.get(*output)?.extremes()?;
                let old = cells.get(slot)?;
                // Fix a realizable source/result pair on one axis before
                // excluding it from the independent old coordinate.
                let first_value = evaluate(first);
                let (source, returned) = if old.singleton_value() == Some(&first_value) && first != last {
                    (last.clone(), evaluate(last))
                } else {
                    (first.clone(), first_value)
                };
                if constrain(cells.get_mut(slot)?, unequal()?, &returned, work)?.is_none() { continue; }
                *cells.get_mut(*output)? = Rc::new(IntegerSet::singleton(&source));
                returned
            }
            (PartitionedProofGoal::EntryAffineEquality { slot, transform }, actual) => {
                let actual = match actual {
                    IntegerPairPartitionOutput::Singleton(values) => AxisProofValue::Literal(values.singleton_value()?),
                    IntegerPairPartitionOutput::InputAlias(axis) if *axis == slot => AxisProofValue::Identity,
                    IntegerPairPartitionOutput::InputAffine(axis, actual) if *axis == slot => AxisProofValue::Affine(actual),
                    _ => return None,
                };
                let cell = cells.get_mut(slot)?;
                let Some((violating, returned)) = rejected_entry_relationship(cell, actual, transform, work)?
                    else { continue; };
                *cell = violating;
                returned
            }
        };
        return Some(PartitionedCallProof { applicable, covers_guard: false, failure: Some(SymbolicLiteralFailure {
            output: piece.span, returned, witness_outside_domain: false,
            state: IntegerInputs::from_cells(vec![Vec::from(cells)]),
            axes: vec![(first, &left.ty), (second, &right.ty)],
        }) });
    }
    Some(PartitionedCallProof { applicable, covers_guard: false, failure: None })
}

#[cfg(feature = "internal-checker")]
enum AxisProofValue<'a> {
    Literal(&'a BigInt),
    Identity,
    Affine(&'a IntegerAffineTransform),
}

/// Solve equality on the same typed entry axis. The returned pair contains
/// actual violating entries and the observed result at their first point.
#[cfg(feature = "internal-checker")]
fn rejected_entry_relationship(
    input: &Rc<IntegerSet>, actual: AxisProofValue<'_>, expected: &IntegerAffineTransform, work: &mut usize,
) -> Option<Option<(Rc<IntegerSet>, BigInt)>> {
    charge_call_work(work, input.interval_count().checked_mul(3)?.checked_add(5)?)?;
    let unequal = IntegerRelation::from_operator(&BinaryOperator::NotEqual)?;
    let rejected = if let AxisProofValue::Literal(value) = actual {
        let (relation, cut) = expected.pull_back_cut(unequal, value);
        Rc::new(input.keep_relation(relation, &cut))
    } else {
        let inverse = expected.inverse();
        let relative = if let AxisProofValue::Affine(actual) = actual { inverse.compose(actual) } else { inverse };
        if relative.identity_everywhere() { return Some(None); }
        if let Some(fixed) = relative.integral_reflection_fixed_point() {
            Rc::new(input.keep_relation(unequal, &fixed))
        } else {
            Rc::clone(input)
        }
    };
    let Some((first, _)) = rejected.extremes() else { return Some(None); };
    let value = match actual {
        AxisProofValue::Literal(value) => value.clone(),
        AxisProofValue::Identity => first.clone(),
        AxisProofValue::Affine(transform) => transform.apply_point(first),
    };
    Some(Some((rejected, value)))
}

#[cfg(feature = "internal-checker")]
pub(super) fn covers_all_partitioned_call_inputs(
    returned: &ActualCallResult, domain: &IntegerSet, work: &mut usize,
) -> Option<bool> {
    let ActualCallResult::PartitionedInteger(pieces) = returned else { return None; };
    unary_partitions_cover(pieces, domain, work)
}

#[cfg(feature = "internal-checker")]
fn unary_partitions_cover(pieces: &[IntegerReturnPartition], domain: &IntegerSet, work: &mut usize) -> Option<bool> {
    charge_call_work(work, pieces.len())?;
    let intervals = pieces.iter().try_fold(0usize, |total, piece| total.checked_add(piece.input.interval_count()))?;
    let sort_depth = (usize::BITS - intervals.leading_zeros()) as usize;
    charge_call_work(work, intervals.checked_mul(sort_depth.checked_mul(2)?.checked_add(4)?)?
        .checked_add(domain.interval_count())?)?;
    Some(domain.is_subset_of(&IntegerSet::union(pieces.iter().map(|piece| piece.input.as_ref()))))
}

#[cfg(feature = "internal-checker")]
pub(super) fn verify_partitioned_call_return<'a>(
    producer: &'a FunctionCallSites, returned: &ActualCallResult, domain: &IntegerSet,
    eligible_for_elision: bool, goal: PartitionedProofGoal<'_>, mut work: Option<&mut usize>,
) -> Option<PartitionedCallProof<'a>> {
    if matches!(goal, PartitionedProofGoal::EntryEquality(slot)
        | PartitionedProofGoal::EntryAffineEquality { slot, .. } if slot != 0)
    {
        return None;
    }
    let ActualCallResult::PartitionedInteger(pieces) = returned else { return None; };
    let [binding] = producer.boundary.checker_parameter_bindings.as_deref()? else { return None; };
    let [parameter] = producer.boundary.parameters.as_slice() else { return None; };
    if let PartitionedProofGoal::EntryAffineEquality { transform, .. } = goal {
        if domain.is_empty() {
            return Some(PartitionedCallProof { applicable: false, covers_guard: false, failure: None });
        }
        let [result] = producer.boundary.results.as_slice() else { return None; };
        if result.ty != parameter.ty { return None; }
        let (min, max) = super::typecheck::integer_bounds(&result.ty)?;
        transform.fits(domain, &min, &max)?;
    }
    let mut applicable = false;
    let mut can_elide = eligible_for_elision;
    let mut covered = Vec::new();
    let mut relational_work = 0;
    for piece in pieces {
        let relational = !matches!(&piece.output, IntegerPartitionOutput::Singleton(_));
        if relational { can_elide = false; }
        let cost = 1usize.checked_add(piece.input.interval_count())?
            .checked_add(domain.interval_count())?;
        if let Some(work) = work.as_deref_mut() {
            charge_call_work(work, cost)?;
        } else if relational {
            charge_call_work(&mut relational_work, cost)?;
        }
        let input = match piece.input.intersection(domain) {
            IntegerIntersection::Unchanged => Rc::clone(&piece.input),
            IntegerIntersection::Empty => continue,
            IntegerIntersection::Changed(clipped) => Rc::new(clipped),
        };
        applicable = true;
        let failure = match (&goal, &piece.output) {
            (PartitionedProofGoal::EntryAffineEquality { transform, .. }, output) => {
                let actual = match output {
                    IntegerPartitionOutput::Singleton(values) => AxisProofValue::Literal(values.singleton_value()?),
                    IntegerPartitionOutput::InputAlias => AxisProofValue::Identity,
                    IntegerPartitionOutput::InputAffine(transform) => AxisProofValue::Affine(transform),
                };
                let meter = work.as_deref_mut().unwrap_or(&mut relational_work);
                rejected_entry_relationship(&input, actual, transform, meter)?
                    .map(|(entries, value)| (entries, Cow::Owned(value)))
            }
            (PartitionedProofGoal::Literal(relation, bound), IntegerPartitionOutput::Singleton(output)) => {
                let value = output.singleton_value()?;
                (!relation.accepts(value.cmp(bound))).then(|| (Rc::clone(&input), Cow::Borrowed(value)))
            }
            (PartitionedProofGoal::EntryEquality(_), IntegerPartitionOutput::Singleton(output)) => {
                let value = output.singleton_value()?;
                if input.singleton_value() == Some(value) { None } else {
                    let unequal = IntegerRelation::from_operator(&BinaryOperator::NotEqual)?;
                    Some((Rc::new(input.keep_relation(unequal, value)), Cow::Borrowed(value)))
                }
            }
            (PartitionedProofGoal::EntryEquality(_), IntegerPartitionOutput::InputAlias) => None,
            (PartitionedProofGoal::EntryEquality(_), IntegerPartitionOutput::InputAffine(transform)) => {
                let meter = work.as_deref_mut().unwrap_or(&mut relational_work);
                charge_call_work(meter, 1usize.checked_add(input.interval_count().checked_mul(2)?)?)?;
                let violating = if transform.identity_everywhere() {
                    None
                } else if let Some(fixed) = transform.integral_reflection_fixed_point() {
                    let unequal = IntegerRelation::from_operator(&BinaryOperator::NotEqual)?;
                    Some(Rc::new(input.keep_relation(unequal, &fixed)))
                } else {
                    Some(Rc::clone(&input))
                };
                violating.and_then(|entries| {
                    let (example, _) = entries.extremes()?;
                    let returned = transform.apply_point(example);
                    Some((entries, Cow::Owned(returned)))
                })
            }
            (PartitionedProofGoal::Literal(relation, bound), output) => {
                let meter = work.as_deref_mut().unwrap_or(&mut relational_work);
                charge_call_work(meter, 1usize.checked_add(input.interval_count().checked_mul(2)?)?)?;
                let rejected = relation.complement();
                let violating = match output {
                    IntegerPartitionOutput::InputAlias => input.keep_relation(rejected, bound),
                    IntegerPartitionOutput::InputAffine(transform) => {
                        let (relation, cut) = transform.pull_back_cut(rejected, bound);
                        input.keep_relation(relation, &cut)
                    }
                    IntegerPartitionOutput::Singleton(_) => return None,
                };
                if let Some((example, _)) = violating.extremes() {
                    let returned = match output {
                        IntegerPartitionOutput::InputAlias => example.clone(),
                        IntegerPartitionOutput::InputAffine(transform) => transform.apply_point(example),
                        IntegerPartitionOutput::Singleton(_) => return None,
                    };
                    Some((Rc::new(violating), Cow::Owned(returned)))
                } else {
                    None
                }
            }
        };
        if let Some((violating_inputs, value)) = failure {
            let violating_inputs = if matches!(goal, PartitionedProofGoal::EntryEquality(_) | PartitionedProofGoal::EntryAffineEquality { .. }) {
                Rc::new(IntegerSet::singleton(violating_inputs.extremes()?.0))
            } else {
                violating_inputs
            };
            return Some(PartitionedCallProof { applicable: true, covers_guard: false, failure: Some(SymbolicLiteralFailure {
                output: piece.span, returned: value.into_owned(), witness_outside_domain: false, state: IntegerInputs::from_cells(vec![vec![violating_inputs]]),
                axes: vec![(binding, &parameter.ty)],
            }) });
        }
        if can_elide {
            covered.push(input);
        }
    }
    let covers_guard = can_elide && applicable
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
        self.path.project_axis_bounded(self.caller_axis, work)
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

#[cfg(feature = "internal-checker")]
pub(super) struct LocalPointeeTransition {
    pub(super) binding: ReferenceBindingId,
    pub(super) ty: ScalarType,
    pub(super) entry: BigInt,
    pub(super) exit: BigInt,
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

#[cfg(feature = "internal-checker")]
#[derive(Clone, Copy)]
enum CallableHistoryOperand<'a> {
    Exact(ExactIntegerArgument<'a>),
    Axis(&'a ScalarType, usize),
}

#[cfg(feature = "internal-checker")]
impl<'a> NestedIntegerArgument<'a> {
    fn callable_history_operands(&self) -> Option<[Option<CallableHistoryOperand<'a>>; 2]> {
        use CallableHistoryOperand::{Axis, Exact};
        Some(match self {
            Self::Single(None) => [None, None],
            Self::Single(Some(arg)) | Self::RootLexical(arg) => [Some(Exact(*arg)), None],
            Self::Pair(left, right) => [Some(Exact(*left)), Some(Exact(*right))],
            Self::Projected(projected) => {
                let here = Some(Axis(projected.ty, projected.caller_axis));
                match (projected.slot, projected.companion) {
                    (0, ProjectedCompanion::Unary) => [here, None],
                    (0, ProjectedCompanion::Fixed(fixed)) => [here, Some(Exact(fixed))],
                    (1, ProjectedCompanion::Fixed(fixed)) => [Some(Exact(fixed)), here],
                    (0, ProjectedCompanion::Independent { ty, axis }) => {
                        let [row] = projected.path.0.as_slice() else { return None; };
                        if row.linked_axes(projected.caller_axis, axis) { return None; }
                        [here, Some(Axis(ty, axis))]
                    }
                    // The caller can hold aliased/affine slots. Do not lift
                    // those into a product of independent child domains.
                    _ => return None,
                }
            }
            _ => return None,
        })
    }

    pub(super) fn callable_history_entry(&self, parameters: &[ScalarType], work: &std::cell::Cell<usize>)
        -> Option<CallableHistoryEntry>
    {
        use CallableHistoryOperand::{Axis, Exact};
        let operands = self.callable_history_operands()?;
        if operands.iter().flatten().count() != parameters.len() { return None; }
        let mut charged = work.get();
        charge_call_work(&mut charged, parameters.len())?;
        let mut domains = Vec::with_capacity(parameters.len());
        for (operand, parameter) in operands.into_iter().flatten().zip(parameters) {
            let domain = match operand {
                Exact((ty, value)) if ty == parameter && super::typecheck::integer_literal_fits_type(value, ty) => {
                    charge_call_work(&mut charged, 1)?;
                    Rc::new(IntegerSet::singleton(value))
                }
                Axis(ty, axis) if ty == parameter => {
                    let Self::Projected(projection) = self else { return None; };
                    let [row] = projection.path.0.as_slice() else { return None; };
                    let values = row.get(axis)?;
                    charge_call_work(&mut charged, values.interval_count())?;
                    Rc::clone(values)
                }
                _ => return None,
            };
            domains.push(domain);
        }
        work.set(charged);
        Some(CallableHistoryEntry(IntegerRow::new(domains)))
    }
}

enum NestedIntegerOperand<'a> {
    Exact(ExactIntegerArgument<'a>),
    Axis(&'a ScalarType, usize),
}

fn nested_integer_operand<'a>(
    calls: &'a FunctionCallSites, expression: OccurrenceOrdinal, slot: usize,
    roots: &IntegerRoots<'_>, path: &'a IntegerInputs, allow_bool: bool, work: Option<&mut usize>,
) -> Option<NestedIntegerOperand<'a>> {
    let (argument, ty) = checked_scalar_call_argument_at(&calls.structure, expression, slot, allow_bool, false)?;
    if let Some(value) = scalar_call_literal(&calls.structure, argument, ty, allow_bool) {
        return Some(NestedIntegerOperand::Exact((ty, value)));
    }
    let PlainNameIdentity::Lexical(binding) = calls.prepared_plain_name(argument)? else { return None; };
    nested_lexical_operand(ty, binding, roots, path, work)
}

fn nested_lexical_operand<'a>(
    ty: &'a ScalarType, binding: &ReferenceBindingId, roots: &IntegerRoots<'_>,
    path: &'a IntegerInputs, work: Option<&mut usize>,
) -> Option<NestedIntegerOperand<'a>> {
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
    classify_nested_pair(first, second, path, allow_projection, allow_equal, allow_independent, work)
}

fn classify_nested_pair<'a>(
    first: NestedIntegerOperand<'a>, second: NestedIntegerOperand<'a>, path: &'a IntegerInputs,
    allow_projection: bool, allow_equal: bool, allow_independent: bool, mut work: Option<&mut usize>,
) -> Option<NestedIntegerArgument<'a>> {
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
    classify_nested_single(
        nested_integer_operand(calls, expression, 0, roots, path, allow_bool, work),
        path, allow_projection,
    )
}

fn classify_nested_single<'a>(
    operand: Option<NestedIntegerOperand<'a>>, path: &'a IntegerInputs, allow_projection: bool,
) -> NestedIntegerArgument<'a> {
    match operand {
        Some(NestedIntegerOperand::Exact(argument)) => NestedIntegerArgument::Single(Some(argument)),
        Some(NestedIntegerOperand::Axis(ty, caller_axis)) if allow_projection =>
            NestedIntegerArgument::Projected(ProjectedIntegerArgument {
                ty, caller_axis, slot: 0, path, companion: ProjectedCompanion::Unary, affine_work: None,
            }),
        _ => NestedIntegerArgument::Single(None),
    }
}

#[cfg(feature = "internal-checker")]
fn ordinary_directive_arguments<'a>(
    operands: &'a [super::reference::CheckerDirectiveCallOperand], parameters: &'a [ScalarType],
    roots: &IntegerRoots<'_>, path: &'a IntegerInputs, unary_projection: bool, pair_projection: bool,
    mut work: Option<&mut usize>,
) -> Option<NestedIntegerArgument<'a>> {
    use super::reference::CheckerDirectiveCallOperand as Operand;
    let mut resolve = |operand: &'a Operand, ty: &'a ScalarType| match operand {
        Operand::Literal(value) => Some(NestedIntegerOperand::Exact((ty, value))),
        Operand::Lexical(binding) => nested_lexical_operand(ty, binding, roots, path, work.as_deref_mut()),
    };
    match (operands, parameters) {
        ([], []) => Some(NestedIntegerArgument::Single(None)),
        ([operand], [ty]) => Some(classify_nested_single(Some(resolve(operand, ty)?), path, unary_projection)),
        ([left, right], [left_ty, right_ty]) => {
            let left = resolve(left, left_ty)?;
            let right = resolve(right, right_ty)?;
            classify_nested_pair(left, right, path, pair_projection, pair_projection, pair_projection, work)
        }
        _ => None,
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
#[derive(Clone, Copy)]
struct ImportedRequirementConditions { comparisons: bool, enum_match: Option<usize> }

impl<'a> ScopedAuthoredReturn<'a> for ImportedRequirementConditions {
    const ENABLED: bool = false;
    #[cfg(feature = "internal-checker")]
    fn allows_imported_condition_comparisons(self) -> bool { self.comparisons }
    fn imported_enum_match_position(self) -> Option<usize> { self.enum_match }
    fn at(
        self, _: &FunctionCallSites, _: BlockOrdinal, _: FinalOutputSlot, _: &ScalarType,
        _: &IntegerRoots<'_>, _: &IntegerInputs,
    ) -> Option<&'a BigInt> { None }
}

pub(super) fn collect_ordered_imported_root_locals<'a>(
    calls: &'a FunctionCallSites, imports: &mut [Option<ImportedLocal<'a>>], prepared_comparisons: bool,
    enum_match: Option<usize>,
    sink: &mut dyn FnMut(IntegerCheckFailure<'a>),
) -> Option<bool> {
    let mut failures = IntegerFailureCollection {
        continuation: IntegerRequirementContinuation::Empty, projected: None, allow_projected_calls: false, allow_contextual_local_transfers: false, sink,
    };
    let ProofCoverage::Applicable(active) = run_integer_callable::<true, true, _>(
        calls, integer_tail_type(calls), None, None, imports, None, None, Some(&mut failures), false, ImportedRequirementConditions { comparisons: prepared_comparisons, enum_match },
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
pub(super) const MAX_FIXED_ARRAY_RETURN_CELLS: usize = MAX_PARTITION_TRANSFER_WORK / 4;

#[cfg(feature = "internal-checker")]
pub(super) struct FixedArrayReturnFailure {
    pub(super) cell: Option<(usize, BigInt)>,
    pub(super) span: super::ByteSpan,
    pub(super) inputs: IntegerInputs,
}

#[cfg(feature = "internal-checker")]
#[derive(Clone, Copy)]
pub(super) enum FixedArrayQuantifier<W> {
    All,
    ExplicitWitness(W),
    FindWitness,
}

#[cfg(feature = "internal-checker")]
impl<W> FixedArrayQuantifier<W> {
    pub(super) fn as_ref(&self) -> FixedArrayQuantifier<&W> {
        match self {
            Self::All => FixedArrayQuantifier::All,
            Self::ExplicitWitness(witness) => FixedArrayQuantifier::ExplicitWitness(witness),
            Self::FindWitness => FixedArrayQuantifier::FindWitness,
        }
    }
}

#[cfg(feature = "internal-checker")]
struct RejectedCell<'a> {
    row: &'a IntegerRow,
    point: Option<(usize, BigInt)>,
    value: BigInt,
}

#[cfg(feature = "internal-checker")]
impl RejectedCell<'_> {
    fn source_inputs(&self, parameters: usize, work: &mut usize) -> Option<IntegerInputs> {
        array_entry_point(self.row, self.point.as_ref(), parameters, work)
    }
}

#[cfg(feature = "internal-checker")]
fn array_entry_point(
    original: &IntegerRow, point: Option<&(usize, BigInt)>, parameters: usize, work: &mut usize,
) -> Option<IntegerInputs> {
    let mut row = realize_integer_row(original, point, 0..parameters, work)?;
    row.truncate(parameters);
    Some(IntegerInputs(vec![row]))
}

#[cfg(feature = "internal-checker")]
fn realize_integer_row(
    original: &IntegerRow, point: Option<&(usize, BigInt)>, axes: impl IntoIterator<Item = usize>, work: &mut usize,
) -> Option<IntegerRow> {
    charge_call_work(work, original.len().checked_add(1)?)?;
    let mut row = original.clone();
    let restrict = |row: &mut IntegerRow, axis, value: &BigInt, work: &mut usize| {
        charge_call_work(work, row.equal_axes().len().checked_add(1)?)?;
        row.charge_affine_replacement(axis, 1, work)?;
        charge_call_work(work, 0)?;
        row.replace_domain(axis, Rc::new(IntegerSet::singleton(value)));
        Some(())
    };
    // Existing row constraints still include hidden captures and aliases.
    // Restrict a concrete rejected cell first when one was selected.
    if let Some((axis, point)) = point {
        restrict(&mut row, *axis, point, work)?;
    }
    for axis in axes {
        charge_call_work(work, 1)?;
        let domain = row.get(axis)?;
        if domain.singleton_value().is_none() {
            let point = domain.extremes()?.0.clone();
            restrict(&mut row, axis, &point, work)?;
        }
    }
    Some(row)
}

#[cfg(feature = "internal-checker")]
struct FixedArrayObservationState<'cursor, 'claims> {
    work: usize,
    failures: Vec<Option<FixedArrayReturnFailure>>,
    applied: Vec<bool>,
    returned: Option<Vec<BigInt>>,
    exact_checks: Option<&'cursor mut [Option<FixedArrayCheck<'claims>>]>,
    symbolic_entries: Option<(usize, IntegerInputs)>,
}

#[cfg(feature = "internal-checker")]
#[derive(Clone, Copy)]
pub(super) struct FixedArrayCheck<'a> {
    pub(super) domain: &'a IntegerSet,
    pub(super) quantifier: FixedArrayQuantifier<&'a BigInt>,
    pub(super) relation: IntegerRelation,
    pub(super) expected: &'a BigInt,
}

#[cfg(feature = "internal-checker")]
impl FixedArrayCheck<'_> {
    fn selects(self, index: &BigInt) -> bool {
        match self.quantifier {
            FixedArrayQuantifier::ExplicitWitness(chosen) => chosen == index,
            FixedArrayQuantifier::All | FixedArrayQuantifier::FindWitness => self.domain.contains(index),
        }
    }

    pub(super) fn applicable(self) -> bool {
        !self.domain.is_empty() || matches!(self.quantifier, FixedArrayQuantifier::FindWitness)
    }
}

#[cfg(feature = "internal-checker")]
#[derive(Clone, Copy)]
pub(super) struct FixedArraySymbolicCheck<'a> {
    pub(super) check: FixedArrayCheck<'a>,
    pub(super) enabled: bool,
    // A claim's original parameter domains, not the marginals of the
    // shared row union. None means the common entry applies as-is.
    pub(super) entry: Option<&'a [IntegerSet]>,
}

#[cfg(feature = "internal-checker")]
#[derive(Clone, Copy)]
enum FixedArrayObservationMode<'a> {
    Symbolic(&'a [FixedArraySymbolicCheck<'a>]),
    Exact,
}

#[cfg(feature = "internal-checker")]
#[derive(Clone, Copy)]
struct FixedArrayObservation<'a, 'cursor, 'claims> {
    mode: FixedArrayObservationMode<'a>,
    state: &'a std::cell::RefCell<FixedArrayObservationState<'cursor, 'claims>>,
    startup_trace: Option<(&'a wosy_syntax::SourceIdentity, &'a super::checker_project::failure_search::SharedStartupTrace)>,
}

#[cfg(feature = "internal-checker")]
impl<'body, 'trace: 'body> ScopedAuthoredReturn<'body> for FixedArrayObservation<'trace, '_, '_> {
    const ENABLED: bool = false;
    const ALLOW_ORDINARY_INTEGER_LOCALS: bool = true;
    const OBSERVE_FIXED_ARRAY: bool = true;
    const REQUIRE_TOTAL_RETURN: bool = true;

    fn allows_extended_scalar_locals(self) -> bool { true }

    fn startup_trace(self) -> Option<(&'body wosy_syntax::SourceIdentity, super::checker_project::failure_search::SharedStartupTrace)> {
        self.startup_trace.map(|(source, trace)| (source, Rc::clone(trace)))
    }

    fn take_fixed_array_entries(self) -> Option<(usize, IntegerInputs)> {
        match self.mode {
            FixedArrayObservationMode::Symbolic(_) => self.state.borrow_mut().symbolic_entries.take(),
            FixedArrayObservationMode::Exact => None,
        }
    }

    fn at(
        self, _: &FunctionCallSites, _: BlockOrdinal, _: FinalOutputSlot, _: &ScalarType,
        _: &IntegerRoots<'_>, _: &IntegerInputs,
    ) -> Option<&'body BigInt> { None }

    fn observe_fixed_array(
        self, calls: &'body FunctionCallSites, slot: FinalOutputSlot, ty: &ScalarType,
        roots: &IntegerRoots<'body>, path: &IntegerInputs,
    ) -> Option<()> {
        use super::checker_directive::CheckerSnapshotNode;
        use super::reference::CheckerArrayReturn;
        let ScalarType::Array { element, length, .. } = ty else { return None; };
        let structure = &calls.structure;
        let (shape, captured) = match structure.checker_array_returns.get(&slot.expression)? {
            CheckerArrayReturn::Constructor(shape) => (shape, false),
            CheckerArrayReturn::Local { constructor } => (structure.checker_snapshot_locals.get(constructor)?.as_ref(), true),
        };
        let CheckerSnapshotNode::Array(cells) = &shape.root else { return None; };
        if u64::try_from(cells.len()).ok()? != *length || cells.len() != shape.leaves.len()
            || cells.len() > MAX_FIXED_ARRAY_RETURN_CELLS || path.is_unreachable()
        {
            return None;
        }
        let mut state = self.state.borrow_mut();
        match self.mode {
            FixedArrayObservationMode::Exact =>
                state.observe_exact(slot, path, roots, shape, cells, captured, element),
            FixedArrayObservationMode::Symbolic(checks) =>
                state.observe_symbolic(slot, checks, path, roots, shape, cells, captured, element),
        }
    }
}

#[cfg(feature = "internal-checker")]
enum FixedArrayRead<'a> {
    Literal(&'a BigInt),
    Axis(usize),
    Arithmetic(AuthoredIntegerOperation, IntegerCalculationOperand<'a>, IntegerCalculationOperand<'a>),
}

#[cfg(feature = "internal-checker")]
impl<'a> FixedArrayRead<'a> {
    fn prepare(
        roots: &IntegerRoots<'_>, leaf: &'a super::checker_directive::CheckerSnapshotLeaf,
        captured: bool, element: &ScalarType,
    ) -> Option<Self> {
        use super::checker_directive::CheckerSnapshotOperand;
        if &leaf.ty != element { return None; }
        let axis = |binding| {
            let (axis, actual) = roots.lookup(binding)?;
            (actual == element).then_some(Self::Axis(axis))
        };
        if captured { return axis(&leaf.capture); }
        match &leaf.operand {
            CheckerSnapshotOperand::Literal(value) => Some(Self::Literal(value)),
            CheckerSnapshotOperand::Binding(binding) => axis(binding),
            CheckerSnapshotOperand::Affine { source, operator, literal, source_on_left } => {
                let (operation, left, right) = checked_snapshot_operation(
                    roots, element, source, operator, literal, *source_on_left,
                )?;
                Some(Self::Arithmetic(operation, left, right))
            }
        }
    }

    fn exact_value(self, row: &IntegerRow, element: &ScalarType) -> Option<BigInt> {
        match self {
            Self::Literal(value) => Some(value.clone()),
            Self::Axis(axis) => row.get(axis)?.singleton_value().cloned(),
            Self::Arithmetic(operation, left, right) => {
                let (min, max) = super::typecheck::integer_bounds(element)?;
                match compute_row_integer_operation(row, operation, left, right, &min, &max)? {
                    ComputedRowInteger::Singleton(value) => Some(value),
                    ComputedRowInteger::Affine(..) => None,
                }
            }
        }
    }
}

#[cfg(feature = "internal-checker")]
impl FixedArrayObservationState<'_, '_> {
    fn observe_exact(
        &mut self, slot: FinalOutputSlot, path: &IntegerInputs,
        roots: &IntegerRoots<'_>, shape: &super::checker_directive::CheckerSnapshotShape,
        cells: &[super::checker_directive::CheckerSnapshotNode], captured: bool, element: &ScalarType,
    ) -> Option<()> {
        use super::checker_directive::CheckerSnapshotNode;
        let [row] = path.0.as_slice() else { return None; };
        let checks = self.exact_checks.take()?; // at most one real return
        let per_cell = checks.iter().try_fold(6usize, |cost, check| {
            cost.checked_add(1)?.checked_add(check.map_or(0, |check| check.domain.interval_count()))
        })?;
        charge_call_work(&mut self.work, cells.len().checked_mul(per_cell)?.checked_add(checks.len())?)?;
        let mut returned = Vec::with_capacity(cells.len());
        for (index, cell) in cells.iter().enumerate() {
            let CheckerSnapshotNode::Leaf(leaf) = cell else { return None; };
            let leaf = shape.leaves.get(*leaf)?;
            let value = FixedArrayRead::prepare(roots, leaf, captured, element)?.exact_value(row, element)?;
            let proof_index = std::cell::OnceCell::new();
            for (open, failure) in checks.iter_mut().zip(&mut self.failures) {
                let Some(check) = open.filter(|check| check.selects(proof_index.get_or_init(|| BigInt::from(index))))
                    else { continue; };
                let passes = check.relation.accepts(value.cmp(check.expected));
                if matches!(check.quantifier, FixedArrayQuantifier::FindWitness) {
                    if passes { *open = None; }
                } else if !passes {
                    *failure = Some(FixedArrayReturnFailure {
                        cell: Some((index, value.clone())), span: leaf.span,
                        inputs: array_entry_point(row, None, roots.parameters, &mut self.work)?,
                    });
                    *open = None;
                }
            }
            // Materialize every cell for the caller, regardless of proof coverage.
            returned.push(value);
        }
        for (check, failure) in checks.iter().zip(&mut self.failures) {
            if check.is_some_and(|check| matches!(check.quantifier, FixedArrayQuantifier::FindWitness)) {
                *failure = Some(FixedArrayReturnFailure {
                    cell: None, span: slot.span,
                    inputs: array_entry_point(row, None, roots.parameters, &mut self.work)?,
                });
            }
        }
        self.returned = Some(returned);
        Some(())
    }

    fn observe_symbolic<'a>(
        &mut self, slot: FinalOutputSlot, checks: &[FixedArraySymbolicCheck<'_>], path: &IntegerInputs,
        roots: &IntegerRoots<'a>, shape: &'a super::checker_directive::CheckerSnapshotShape,
        cells: &[super::checker_directive::CheckerSnapshotNode], captured: bool, element: &ScalarType,
    ) -> Option<()> {
        use super::checker_directive::CheckerSnapshotNode;
        charge_call_work(&mut self.work, checks.len())?;
        let per_cell = checks.iter().try_fold(1usize, |cost, claim| {
            cost.checked_add(1)?.checked_add(claim.check.domain.interval_count())
        })?;
        charge_call_work(&mut self.work, cells.len().checked_mul(per_cell)?)?;
        let mut claims = Vec::with_capacity(checks.len());
        for ((claim, prior), applied) in checks.iter().zip(&self.failures).zip(&mut self.applied) {
            let open = prior.is_none() && claim.enabled && claim.check.applicable();
            let inferred = matches!(claim.check.quantifier, FixedArrayQuantifier::FindWitness);
            let rows = if open && (inferred || claim.entry.is_some()) {
                Some(UnwitnessedArrayRows::new(&path.0, claim.entry, &mut self.work)?)
            } else { None };
            let open = open && rows.as_ref().is_none_or(|set| set.left != 0);
            *applied |= open;
            claims.push(FixedArraySymbolicClaim { check: claim.check, open, inferred, selected: false, rows });
        }
        for (index, cell) in cells.iter().enumerate() {
            let CheckerSnapshotNode::Leaf(leaf) = cell else { return None; };
            let leaf = shape.leaves.get(*leaf)?;
            let read = FixedArrayRead::prepare(roots, leaf, captured, element)?;
            let mut selected = 0usize;
            let proof_index = std::cell::OnceCell::new();
            for claim in &mut claims {
                claim.selected = claim.open && (!claim.inferred || claim.rows.as_ref()?.left != 0)
                    && claim.check.selects(proof_index.get_or_init(|| BigInt::from(index)));
                selected += usize::from(claim.selected);
            }
            match read {
                FixedArrayRead::Literal(value) if selected != 0 => {
                    for (claim, failure) in claims.iter_mut().zip(&mut self.failures) {
                        if !claim.selected { continue; }
                        if claim.check.relation.accepts(value.cmp(claim.check.expected)) {
                            if claim.inferred { claim.rows.as_mut()?.left = 0; }
                        } else if !claim.inferred {
                            if let Some(rows) = &claim.rows { charge_call_work(&mut self.work, rows.rows.len())?; }
                            let inputs = array_entry_point(claim.first_row(&path.0)?, None, roots.parameters, &mut self.work)?;
                            *failure = Some(FixedArrayReturnFailure { cell: Some((index, value.clone())), span: leaf.span, inputs });
                            claim.open = false;
                        }
                    }
                }
                FixedArrayRead::Axis(axis) if selected != 0 => {
                    charge_call_work(&mut self.work, path.0.len().checked_mul(claims.len())?)?;
                    for (row_index, row) in path.0.iter().enumerate() {
                        self.check_symbolic_row(&mut claims, (row_index, row), FixedArrayCellValue::Axis(axis, None),
                            (index, leaf.span), roots.parameters)?;
                    }
                }
                FixedArrayRead::Arithmetic(operation, left, right) => {
                    let (min, max) = super::typecheck::integer_bounds(&leaf.ty)?;
                    charge_call_work(&mut self.work, path.0.len().checked_mul(5usize.checked_add(
                        if selected == 0 { 0 } else { claims.len() }
                    )?)?)?;
                    for (row_index, row) in path.0.iter().enumerate() {
                        // Validate on the canonical union row even if no active
                        // goal selects this cell. One computation feeds all claims.
                        let value = compute_row_integer_operation(row, operation, left, right, &min, &max)?;
                        if selected != 0 {
                            let evidence = match &value {
                                ComputedRowInteger::Singleton(value) => FixedArrayCellValue::Point(value),
                                ComputedRowInteger::Affine(axis, transform) => FixedArrayCellValue::Axis(*axis, Some(transform)),
                            };
                            self.check_symbolic_row(&mut claims, (row_index, row), evidence, (index, leaf.span), roots.parameters)?;
                        }
                    }
                }
                FixedArrayRead::Literal(_) | FixedArrayRead::Axis(_) => {}
            }
        }
        for (claim, failure) in claims.into_iter().zip(&mut self.failures) {
            if claim.open && claim.inferred {
                let rows = claim.rows.as_ref()?;
                if rows.left != 0 {
                    charge_call_work(&mut self.work, rows.rows.len())?;
                    let inputs = array_entry_point(claim.first_row(&path.0)?, None, roots.parameters, &mut self.work)?;
                    *failure = Some(FixedArrayReturnFailure { cell: None, span: slot.span, inputs });
                }
            }
        }
        Some(())
    }

    fn check_symbolic_row<'row>(
        &mut self, claims: &mut [FixedArraySymbolicClaim<'_, 'row>], (index, row): (usize, &'row IntegerRow),
        value: FixedArrayCellValue<'_>, cell: (usize, super::ByteSpan), parameters: usize,
    ) -> Option<()> {
        for (claim, failure) in claims.iter_mut().zip(&mut self.failures) {
            if claim.selected && claim.open {
                claim.observe(index, row, value, cell, failure, parameters, &mut self.work)?;
            }
        }
        Some(())
    }
}

#[cfg(feature = "internal-checker")]
#[derive(Clone, Copy)]
enum FixedArrayCellValue<'a> {
    Point(&'a BigInt),
    Axis(usize, Option<&'a IntegerAffineTransform>),
}

#[cfg(feature = "internal-checker")]
struct FixedArraySymbolicClaim<'claim, 'row> {
    check: FixedArrayCheck<'claim>,
    open: bool,
    inferred: bool,
    selected: bool,
    rows: Option<UnwitnessedArrayRows<'row>>,
}

#[cfg(feature = "internal-checker")]
impl<'row> FixedArraySymbolicClaim<'_, 'row> {
    fn first_row<'a>(&'a self, path: &'row [IntegerRow]) -> Option<&'a IntegerRow> where 'row: 'a {
        match &self.rows {
            Some(rows) => rows.rows.iter().flatten().next().map(Cow::as_ref),
            None => path.first(),
        }
    }

    fn observe(
        &mut self, row_index: usize, canonical: &'row IntegerRow, value: FixedArrayCellValue<'_>,
        (index, span): (usize, super::ByteSpan), failure: &mut Option<FixedArrayReturnFailure>,
        parameters: usize, work: &mut usize,
    ) -> Option<()> {
        let row = if let Some(rows) = &self.rows {
            let Some(row) = rows.rows.get(row_index)?.as_ref() else { return Some(()); };
            row.as_ref()
        } else { canonical };
        let rejected = self.check.relation.complement();
        let rejected = match value {
            FixedArrayCellValue::Point(value) => {
                if self.inferred {
                    if !rejected.accepts(value.cmp(self.check.expected)) { self.rows.as_mut()?.remove(row_index); }
                    return Some(());
                }
                rejected.accepts(value.cmp(self.check.expected)).then(|| RejectedCell { row, point: None, value: value.clone() })
            }
            FixedArrayCellValue::Axis(axis, transform) => {
                let (relation, bound) = transform.map_or_else(
                    || (rejected, Cow::Borrowed(self.check.expected)), |transform| {
                        let (relation, bound) = transform.pull_back_cut(rejected, self.check.expected);
                        (relation, Cow::Owned(bound))
                    });
                let cut = FixedArrayCut { axis, relation, bound, transform };
                if self.inferred {
                    return self.rows.as_mut()?.retain_rejected(row_index, &cut, work);
                }
                cut.first_rejected(row, work)?
            }
        };
        if let Some(rejected) = rejected {
            let inputs = rejected.source_inputs(parameters, work)?;
            *failure = Some(FixedArrayReturnFailure { cell: Some((index, rejected.value)), span, inputs });
            self.open = false;
        }
        Some(())
    }
}

#[cfg(feature = "internal-checker")]
struct FixedArrayCut<'a> {
    axis: usize,
    relation: IntegerRelation,
    bound: Cow<'a, BigInt>,
    transform: Option<&'a IntegerAffineTransform>,
}

#[cfg(feature = "internal-checker")]
impl FixedArrayCut<'_> {
    fn first_rejected<'a>(&self, row: &'a IntegerRow, work: &mut usize) -> Option<Option<RejectedCell<'a>>> {
        let domain = row.get(self.axis)?;
        charge_call_work(work, domain.interval_count())?;
        Some(domain.first_comparison_point(self.relation, &self.bound).map(|point| {
            let value = self.transform.as_ref().map_or_else(|| point.clone(), |transform| transform.apply_point(point));
            RejectedCell { row, point: Some((self.axis, point.clone())), value }
        }))
    }
}

// These are the entries on one return path at which every candidate tested
// so far fails. Narrow each entry row with the same scalar cuts used to
// refute a specific cell; never take a cross-product of cell marginals.
#[cfg(feature = "internal-checker")]
struct UnwitnessedArrayRows<'a> {
    left: usize,
    rows: Vec<Option<Cow<'a, IntegerRow>>>,
}

#[cfg(feature = "internal-checker")]
impl<'a> UnwitnessedArrayRows<'a> {
    fn new(path: &'a [IntegerRow], entry: Option<&[IntegerSet]>, work: &mut usize) -> Option<Self> {
        charge_call_work(work, path.len())?;
        let mut rows = Vec::with_capacity(path.len());
        let mut left = 0;
        'sources: for original in path {
            let mut row = Cow::Borrowed(original);
            for (axis, entry) in entry.into_iter().flatten().enumerate() {
                let domain = row.get(axis)?;
                charge_call_work(work, domain.interval_count().checked_add(entry.interval_count())?.checked_add(1)?)?;
                match domain.intersection(entry) {
                    IntegerIntersection::Unchanged => {}
                    IntegerIntersection::Empty => { rows.push(None); continue 'sources; }
                    IntegerIntersection::Changed(set) => {
                        let copy = if matches!(row, Cow::Borrowed(_)) { row.len() } else { 0 };
                        charge_call_work(work, copy.checked_add(row.equal_axes().len())?)?;
                        row.charge_affine_replacement(axis, set.interval_count(), work)?;
                        charge_call_work(work, 0)?;
                        row.to_mut().replace_domain(axis, Rc::new(set));
                    }
                }
            }
            left += 1;
            rows.push(Some(row));
        }
        Some(Self { rows, left })
    }

    fn remove(&mut self, index: usize) {
        debug_assert!(self.rows[index].is_some());
        self.rows[index] = None;
        self.left -= 1;
    }

    fn retain_rejected(&mut self, index: usize, cut: &FixedArrayCut<'_>, work: &mut usize) -> Option<()> {
        if self.left == 0 { return Some(()); }
        let Some(row) = self.rows.get_mut(index)?.as_mut() else { return Some(()); };
        let domain = row.get(cut.axis)?;
        charge_call_work(work, domain.interval_count().checked_mul(3)?.checked_add(1)?)?;
        let remaining = domain.keep_relation(cut.relation, &cut.bound);
        if remaining.is_empty() {
            self.remove(index);
        } else if remaining != **domain {
            let copy = if matches!(row, Cow::Borrowed(_)) { row.len() } else { 0 };
            charge_call_work(work, copy.checked_add(row.equal_axes().len())?)?;
            row.charge_affine_replacement(cut.axis, remaining.interval_count(), work)?;
            charge_call_work(work, 0)?;
            row.to_mut().replace_domain(cut.axis, Rc::new(remaining));
        }
        Some(())
    }
}

#[cfg(all(test, feature = "internal-checker"))]
thread_local! {
    static ARRAY_RETURN_OBSERVATIONS: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}

#[cfg(all(test, feature = "internal-checker"))]
pub(super) fn take_array_return_observations() -> (usize, usize) {
    ARRAY_RETURN_OBSERVATIONS.with(|observations| observations.replace((0, 0)))
}

#[cfg(feature = "internal-checker")]
pub(super) enum FixedArrayEntries<'a> {
    Common(&'a [CheckerPredicate]),
    Guarded { arity: usize, rectangles: &'a [[IntegerSet; 2]] },
}

/// The exact union of the authored entry rectangles. A first-axis edge
/// changes which guards are active, so each slice can use only the second
/// axes of those active rectangles. Emitted slices do not overlap.
#[cfg(feature = "internal-checker")]
fn fixed_array_entry_rows(arity: usize, rectangles: &[[IntegerSet; 2]], work: &mut usize) -> Option<IntegerInputs> {
    let sorted_work = |count: usize| {
        let order = (usize::BITS - count.leading_zeros()) as usize;
        count.checked_mul(order + 2)
    };
    charge_call_work(work, rectangles.len())?;
    if arity == 1 {
        let intervals = rectangles.iter().try_fold(0usize, |total, domains| total.checked_add(domains[0].interval_count()))?;
        charge_call_work(work, sorted_work(intervals)?)?;
        let domain = IntegerSet::union(rectangles.iter().map(|domains| &domains[0]));
        return Some(if domain.is_empty() { IntegerInputs::default() }
            else { IntegerInputs(vec![IntegerRow::new(vec![Rc::new(domain)])]) });
    }
    if arity != 2 { return None; }
    let edges = rectangles.iter().filter(|domains| !domains[1].is_empty())
        .try_fold(0usize, |total, domains| total.checked_add(domains[0].interval_count().checked_mul(2)?))?;
    charge_call_work(work, sorted_work(edges)?.checked_mul(2)?)?;
    let mut events = Vec::with_capacity(edges);
    for (id, [first, second]) in rectangles.iter().enumerate() {
        if second.is_empty() { continue; }
        for (low, high) in first.intervals() {
            events.push((low.clone(), id, true));
            // This is only the open upper edge, never a value in a row.
            events.push((high + 1, id, false));
        }
    }
    events.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    let mut events = events.into_iter().peekable();
    let mut active = std::collections::BTreeSet::<usize>::new();
    let mut start = None;
    let mut slices: Vec<[IntegerSet; 2]> = Vec::new();
    while let Some((edge, id, add)) = events.next() {
        if let Some(first) = start.replace(edge.clone()).filter(|_| !active.is_empty()) {
            charge_call_work(work, active.len())?;
            let intervals = active.iter().try_fold(0usize, |total, &id| {
                total.checked_add(rectangles.get(id)?[1].interval_count())
            })?;
            charge_call_work(work, sorted_work(intervals)?.checked_add(2)?)?;
            let second = IntegerSet::union(active.iter().map(|&id| &rectangles[id][1]));
            let first = IntegerSet::closed_interval(first, &edge - 1)?;
            if let Some([prior_first, prior_second]) = slices.last_mut() {
                charge_call_work(work, prior_second.interval_count().checked_add(second.interval_count())?)?;
                if *prior_second == second {
                    charge_call_work(work, prior_first.interval_count().checked_add(1)?)?;
                    *prior_first = std::mem::take(prior_first).join(first);
                } else {
                    slices.push([first, second]);
                }
            } else {
                slices.push([first, second]);
            }
        }
        let mut record = |id, add| if add { active.insert(id) } else { active.remove(&id) };
        if !record(id, add) { return None; }
        while events.peek().is_some_and(|event| event.0 == edge) {
            let (_, id, add) = events.next()?;
            if !record(id, add) { return None; }
        }
    }
    if !active.is_empty() { return None; }
    charge_call_work(work, slices.len().checked_mul(3)?)?;
    Some(IntegerInputs(slices.into_iter().map(|row|
        IntegerRow::new(row.into_iter().map(Rc::new).collect())).collect()))
}

#[cfg(all(test, feature = "internal-checker"))]
#[test]
fn fixed_array_rectangle_union_has_exactly_one_row_for_each_covered_point() {
    let closed = |first: i16, last: i16| IntegerSet::closed_interval(first.into(), last.into()).unwrap();
    let first = closed(-2, 1).join(closed(5, 5));
    let second = closed(-1, 0).join(closed(2, 2));
    let rectangles = [
        [first, second],
        [closed(0, 4), closed(1, 2)],
        [closed(4, 7), closed(-2, 0)],
        [closed(1, 3), IntegerSet::default()],
        [closed(3, 4), closed(1, 2)],
        [closed(255, 255), closed(4, 4)],
    ];
    let mut work = 0;
    let rows = fixed_array_entry_rows(2, &rectangles, &mut work).unwrap();
    for x in -3..=256i16 {
        for y in -3..=5i16 {
            let [x, y] = [BigInt::from(x), BigInt::from(y)];
            let expected = rectangles.iter().any(|[a, b]| a.contains(&x) && b.contains(&y));
            let times = rows.0.iter().filter(|row| row[0].contains(&x) && row[1].contains(&y)).count();
            assert_eq!(times, usize::from(expected), "coverage at ({x}, {y})");
        }
    }
    assert!(fixed_array_entry_rows(2, &[[closed(1, 2), IntegerSet::default()]], &mut 0)
        .unwrap().is_unreachable());
    let mut almost_exhausted = MAX_PARTITION_TRANSFER_WORK - 3;
    assert!(fixed_array_entry_rows(2, &rectangles, &mut almost_exhausted).is_none());
}

/// Read-only consumer of the same prepared blocks, local capture axes and
/// branch partitions as ordinary scalar checking. No aggregate summary is issued.
#[cfg(feature = "internal-checker")]
pub(super) fn observe_fixed_array_returns(
    calls: &FunctionCallSites, result: &ScalarType, entry: FixedArrayEntries<'_>,
    checks: &[FixedArraySymbolicCheck<'_>], work: &mut usize,
) -> Option<Vec<ProofCoverage<Option<FixedArrayReturnFailure>>>> {
    if !matches!(result, ScalarType::Array { .. }) { return None; }
    charge_call_work(work, checks.len())?;
    let (entry_filter, symbolic_entries) = match entry {
        FixedArrayEntries::Common(filter) => (Some(filter), None),
        FixedArrayEntries::Guarded { arity, rectangles } =>
            (None, Some((arity, fixed_array_entry_rows(arity, rectangles, work)?))),
    };
    let fresh = || (0..checks.len()).map(|_| None).collect();
    let state = std::cell::RefCell::new(FixedArrayObservationState {
        work: *work, failures: fresh(), applied: vec![false; checks.len()], returned: None,
        exact_checks: None, symbolic_entries,
    });
    let observer = FixedArrayObservation { mode: FixedArrayObservationMode::Symbolic(checks), state: &state, startup_trace: None };
    #[cfg(test)]
    ARRAY_RETURN_OBSERVATIONS.with(|count| { let (symbolic, actual) = count.get(); count.set((symbolic + 1, actual)); });
    run_integer_callable::<false, false, _>(
        calls, Some(result), entry_filter, None, &mut [], None, None, None, false, observer,
    )?.ok()?;
    let state = state.into_inner();
    *work = state.work;
    Some(state.applied.into_iter().zip(state.failures).map(|(applied, failure)| {
        if applied { ProofCoverage::Applicable(failure) } else { ProofCoverage::NoApplicableInputs }
    }).collect())
}

/// One real entry and one canonical walk. The materialized value feeds the
/// caller and every applicable proof; failed proofs supply no authority.
#[cfg(feature = "internal-checker")]
pub(super) fn collect_exact_fixed_array_return<'claims>(
    calls: &FunctionCallSites, result: &ScalarType, arguments: &[&BigInt], checks: &mut [Option<FixedArrayCheck<'claims>>],
    work: &mut usize,
    startup_trace: Option<(&wosy_syntax::SourceIdentity, &super::checker_project::failure_search::SharedStartupTrace)>,
) -> Option<(Vec<BigInt>, Vec<Option<FixedArrayReturnFailure>>)> {
    let state = std::cell::RefCell::new(FixedArrayObservationState {
        work: *work, failures: (0..checks.len()).map(|_| None).collect(), applied: Vec::new(),
        returned: None, exact_checks: Some(checks), symbolic_entries: None,
    });
    let observer = FixedArrayObservation { mode: FixedArrayObservationMode::Exact, state: &state, startup_trace };
    #[cfg(test)]
    ARRAY_RETURN_OBSERVATIONS.with(|count| { let (symbolic, actual) = count.get(); count.set((symbolic, actual + 1)); });
    let ProofCoverage::Applicable(_) = run_integer_callable::<false, false, _>(
        calls, Some(result), None, None, &mut [], Some(arguments), None, None, false, observer,
    )?.ok()? else { return None; };
    let state = state.into_inner();
    *work = state.work;
    Some((state.returned?, state.failures))
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

#[cfg(feature = "internal-checker")]
pub(super) fn is_result_pair_atom(ty: &ScalarType) -> bool {
    is_integer_type(ty) || matches!(ty, ScalarType::Bool | ScalarType::Char)
}

// The resolved block contains two separate, positional terminal expressions.
// A single expression that projects two results does not have this shape.
#[cfg(feature = "internal-checker")]
pub(super) fn direct_result_pair(calls: &FunctionCallSites) -> Option<[(&FinalOutputSlot, &ScalarType); 2]> {
    let structure = &calls.structure;
    let [first, second] = calls.boundary.results.as_slice() else { return None; };
    let [first_type, second_type] = calls.boundary.root_outputs.as_slice() else { return None; };
    let [.., first_item, second_item] = structure.blocks.get(structure.root.0)?.items.as_slice() else { return None; };
    let (BlockItemKind::FinalOutput(left), BlockItemKind::FinalOutput(right)) = (&first_item.kind, &second_item.kind)
        else { return None; };
    let pairs = [(left, &first.ty), (right, &second.ty)];
    if [first_type, second_type].into_iter().zip(pairs).enumerate().all(|(index, (root, (slot, ty)))|
        root.output == index && slot.output == index && &root.ty == ty
            && is_result_pair_atom(ty)
            && structure.checked_types.expressions.get(&slot.expression) == Some(ty)
            && !calls.conditionals.contains_key(&slot.expression)
            && !structure.checker_direct_targets.contains_key(&slot.expression))
    {
        Some(pairs)
    } else {
        None
    }
}

#[cfg(feature = "internal-checker")]
pub(super) enum ResultPairTerm {
    Slot(usize),
    Literal(BigInt),
}

#[cfg(feature = "internal-checker")]
pub(super) enum ResultPairBoolean {
    Constant(bool),
    Compare { operands: [ResultPairTerm; 2], relation: IntegerRelation },
    Not(Box<Self>),
    Binary { and: bool, operands: Box<[Self; 2]> },
}

#[cfg(feature = "internal-checker")]
pub(super) enum ResultPairClause {
    Literal(ResultSlotLiteral),
    Joint { predicate: ResultPairBoolean, slot: Option<usize> },
}

#[cfg(feature = "internal-checker")]
pub(super) struct ResultSlotLiteral {
    pub(super) slot: usize,
    pub(super) relation: IntegerRelation,
    pub(super) bound: BigInt,
}

#[cfg(feature = "internal-checker")]
impl ResultPairClause {
    fn slot(&self) -> Option<usize> {
        match self {
            Self::Literal(literal) => Some(literal.slot),
            Self::Joint { slot, .. } => *slot,
        }
    }

    pub(super) fn literal(&self) -> Option<&ResultSlotLiteral> {
        let Self::Literal(literal) = self else { return None; };
        Some(literal)
    }
}

#[cfg(feature = "internal-checker")]
pub(super) struct ResultPairProof {
    pub(super) terminal: ResultPairClause,
    pub(super) show: Option<(super::ByteSpan, ResultPairClause)>,
    pub(super) same_literal: bool,
}

#[cfg(feature = "internal-checker")]
impl ResultPairProof {
    fn legacy(&self) -> Option<(&ResultSlotLiteral, Option<(super::ByteSpan, &ResultSlotLiteral)>)> {
        let show = match &self.show {
            Some((span, claim)) => Some((*span, claim.literal()?)),
            None => None,
        };
        Some((self.terminal.literal()?, show))
    }
}

#[cfg(feature = "internal-checker")]
pub(super) struct ResultPairFailure<'a> {
    pub(super) step: Option<super::ByteSpan>,
    pub(super) source_slot: Option<usize>,
    pub(super) witness: Option<SymbolicResultPairWitness<'a>>,
}

#[cfg(feature = "internal-checker")]
pub(super) enum SymbolicResultPairWitness<'a> {
    Literal(SymbolicLiteralFailure<'a>),
    Joint {
        spans: [super::ByteSpan; 2],
        values: [BigInt; 2],
        entry: IntegerInputs,
        axes: Vec<(&'a ReferenceBindingId, &'a ScalarType)>,
    },
}

#[cfg(feature = "internal-checker")]
pub(super) struct ResultPairCheck<'a, 'body> {
    pub(super) proof: &'a ResultPairProof,
    pub(super) admitted: bool,
    pub(super) failure: Option<ResultPairFailure<'body>>,
}

#[cfg(feature = "internal-checker")]
pub(super) enum ResultPairEntry<'a> {
    Exact(&'a [&'a BigInt]),
    Symbolic(&'a [CheckerPredicate]),
}

#[cfg(feature = "internal-checker")]
struct ResultPairState<'plan, 'a, 'checks> {
    checks: &'checks mut [ResultPairCheck<'plan, 'a>],
    legacy: [Vec<SymbolicLiteralCheck<'plan, 'a>>; 2],
    exact: bool,
    returned: Option<[BigInt; 2]>,
    observed: bool,
    work: usize,
}

#[cfg(feature = "internal-checker")]
#[derive(Clone, Copy)]
struct ResultPairSink<'plan, 'body, 'checks, 'run>(&'run std::cell::RefCell<ResultPairState<'plan, 'body, 'checks>>);

#[cfg(feature = "internal-checker")]
#[derive(Clone, Copy)]
enum PreparedPairValue<'a> {
    Axis(usize),
    Literal(&'a BigInt),
}

#[cfg(feature = "internal-checker")]
impl<'a> PreparedPairValue<'a> {
    fn resolve(term: &'a ResultPairTerm, outputs: &[Self; 2]) -> Option<Self> {
        match term {
            ResultPairTerm::Slot(slot) => outputs.get(*slot).copied(),
            ResultPairTerm::Literal(value) => Some(Self::Literal(value)),
        }
    }

    fn point<'row>(&'row self, row: &'row IntegerRow) -> Option<&'row BigInt> {
        match self {
            Self::Literal(value) => Some(value),
            Self::Axis(axis) => row.get(*axis)?.singleton_value(),
        }
    }
}

#[cfg(feature = "internal-checker")]
impl ResultPairTerm {
    fn exact<'a>(&'a self, points: &'a [BigInt; 2]) -> Option<&'a BigInt> {
        match self {
            Self::Slot(slot) => points.get(*slot),
            Self::Literal(value) => Some(value),
        }
    }
}

#[cfg(feature = "internal-checker")]
impl ResultPairBoolean {
    fn exact(&self, points: &[BigInt; 2], work: &mut usize) -> Option<bool> {
        charge_call_work(work, 1)?;
        Some(match self {
            Self::Constant(value) => *value,
            Self::Compare { operands, relation } => relation.accepts(operands[0].exact(points)?.cmp(operands[1].exact(points)?)),
            Self::Not(value) => !value.exact(points, work)?,
            Self::Binary { and, operands } => {
                let left = operands[0].exact(points, work)?;
                if left == *and { operands[1].exact(points, work)? } else { left }
            }
        })
    }

    // A visitor may stop on the first witnessed row. Left and right are
    // checked on the same narrowed row, never on independent marginals.
    fn visit(
        &self, row: &IntegerRow, outputs: &[PreparedPairValue<'_>; 2], selected: [bool; 2], work: &mut usize,
        accept: &mut dyn FnMut(Cow<'_, IntegerRow>, usize, &mut usize) -> Option<bool>,
    ) -> Option<bool> {
        charge_call_work(work, 1)?;
        match self {
            Self::Constant(value) => visit_pair_constant(row, *value, selected, work, accept),
            Self::Compare { operands, relation } => {
                let [left, right] = [PreparedPairValue::resolve(&operands[0], outputs)?,
                    PreparedPairValue::resolve(&operands[1], outputs)?];
                let atom = match (left, right) {
                    (PreparedPairValue::Literal(left), PreparedPairValue::Literal(right)) =>
                        return visit_pair_constant(row, relation.accepts(left.cmp(right)), selected, work, accept),
                    (PreparedPairValue::Axis(axis), PreparedPairValue::Literal(bound)) =>
                        OrdinaryIntegerAtom::Literal(axis, *relation, bound),
                    (PreparedPairValue::Literal(bound), PreparedPairValue::Axis(axis)) =>
                        OrdinaryIntegerAtom::Literal(axis, relation.reversed(), bound),
                    (PreparedPairValue::Axis(left), PreparedPairValue::Axis(right)) =>
                        OrdinaryIntegerAtom::Lexical([left, right], *relation),
                };
                IntegerInputs::visit_ordinary_atom::<true>(row, &atom, selected, work, &mut |row, lane, work| accept(row, lane, work))
            }
            Self::Not(value) => value.visit(row, outputs, [selected[1], selected[0]], work,
                &mut |row, lane, work| accept(row, 1 - lane, work)),
            Self::Binary { and, operands } => {
                let continuing = usize::from(*and);
                let mut left = selected;
                left[continuing] = true;
                operands[0].visit(row, outputs, left, work, &mut |row, lane, work| {
                    if lane == continuing { operands[1].visit(&row, outputs, selected, work, accept) }
                    else { accept(row, lane, work) }
                })
            }
        }
    }
}

#[cfg(feature = "internal-checker")]
fn visit_pair_constant(
    row: &IntegerRow, value: bool, selected: [bool; 2], work: &mut usize,
    accept: &mut dyn FnMut(Cow<'_, IntegerRow>, usize, &mut usize) -> Option<bool>,
) -> Option<bool> {
    if selected[value as usize] { accept(Cow::Borrowed(row), value as usize, work) } else { Some(false) }
}

#[cfg(feature = "internal-checker")]
impl ResultPairProof {
    fn exact_failure(&self, points: &[BigInt; 2], work: &mut usize) -> Option<Option<ResultPairFailure<'static>>> {
        if let Some((terminal, show)) = self.legacy() {
            // Legacy exact comparisons were already prepaid by enrollment.
            let order = if let Some((span, show)) = show {
                let order = points.get(show.slot)?.cmp(&show.bound);
                if !show.relation.accepts(order) {
                    return Some(Some(ResultPairFailure { step: Some(span), source_slot: Some(show.slot), witness: None }));
                }
                if self.same_literal { order } else { points.get(terminal.slot)?.cmp(&terminal.bound) }
            } else {
                points.get(terminal.slot)?.cmp(&terminal.bound)
            };
            return Some((!terminal.relation.accepts(order)).then_some(ResultPairFailure {
                step: None, source_slot: Some(terminal.slot), witness: None,
            }));
        }
        for (clause, step) in self.clauses() {
            let valid = match clause {
                ResultPairClause::Literal(literal) => {
                    charge_call_work(work, 1)?;
                    literal.relation.accepts(points.get(literal.slot)?.cmp(&literal.bound))
                }
                ResultPairClause::Joint { predicate, .. } => predicate.exact(points, work)?,
            };
            if !valid {
                return Some(Some(ResultPairFailure { step, source_slot: clause.slot(), witness: None }));
            }
        }
        Some(None)
    }

    fn clauses(&self) -> impl Iterator<Item = (&ResultPairClause, Option<super::ByteSpan>)> {
        self.show.as_ref().into_iter().map(|(span, claim)| (claim, Some(*span)))
            .chain(std::iter::once((&self.terminal, None)))
    }
}

#[cfg(feature = "internal-checker")]
impl<'body> ScopedAuthoredReturn<'body> for ResultPairSink<'_, 'body, '_, '_> {
    const ENABLED: bool = false;
    const ALLOW_ORDINARY_INTEGER_LOCALS: bool = true;
    const OBSERVE_RESULT_PAIR: bool = true;
    const REQUIRE_TOTAL_RETURN: bool = true;

    fn allows_extended_scalar_locals(self) -> bool { true }

    fn at(
        self, _: &FunctionCallSites, _: BlockOrdinal, _: FinalOutputSlot, _: &ScalarType,
        _: &IntegerRoots<'_>, _: &IntegerInputs,
    ) -> Option<&'body BigInt> { None }

    fn observe_result_pair(
        self, calls: &'body FunctionCallSites, block: BlockOrdinal, slot: FinalOutputSlot,
        roots: &IntegerRoots<'body>, path: &IntegerInputs,
    ) -> Option<()> {
        if block != calls.structure.root { return None; }
        let [(first, first_ty), (second, second_ty)] = direct_result_pair(calls)?;
        if *first != slot { return None; }
        let prepare = |slot, ty| match plain_typed_return(calls, slot, ty, roots)? {
            IntegerReturn::Const(value) => Some(PreparedPairValue::Literal(value)),
            IntegerReturn::Alias(binding) => Some(PreparedPairValue::Axis(roots.lookup(binding)?.0)),
        };
        let values = [prepare(first, first_ty)?, prepare(second, second_ty)?];
        let spans = [first.span, second.span];
        let points = |row: &IntegerRow| Some([values[0].point(row)?.clone(), values[1].point(row)?.clone()]);
        let mut state = self.0.borrow_mut();
        let ResultPairState { checks, legacy, exact, returned, observed, work } = &mut *state;
        if *observed { return None; }
        *observed = true;
        if *exact {
            let [row] = path.0.as_slice() else { return None; };
            let points = points(row)?;
            for check in checks.iter_mut().filter(|check| check.admitted) {
                check.failure = check.proof.exact_failure(&points, work)?;
            }
            *returned = Some(points);
            return Some(());
        }
        let mut literal_work = 0;
        for ((slot, value), bucket) in [(*first, values[0]), (*second, values[1])].into_iter().zip(legacy) {
            if bucket.is_empty() { continue; }
            // Retain the exact original batched symbolic accounting and
            // witness selection for every prior single-slot obligation.
            let value = match value {
                PreparedPairValue::Literal(value) => IntegerReturn::Const(value),
                PreparedPairValue::Axis(axis) => IntegerReturn::Alias(roots.axes.get(axis)?.0),
            };
            let mut observed = BatchedLiteralEvidence { returned: false, work: literal_work, pending: bucket.len(), checks: bucket };
            observe_batched_literal_returns::<false, true>(&mut observed, slot, value, roots, path)?;
            literal_work = observed.work;
        }
        for check in checks.iter_mut().filter(|check| check.admitted && check.proof.legacy().is_none()) {
            for (clause, step) in check.proof.clauses() {
                let mut found = None;
                let mut accept = |row: Cow<'_, IntegerRow>, _: usize, work: &mut usize| {
                    let selected = values.iter().filter_map(|value| match *value {
                        PreparedPairValue::Axis(axis) => Some(axis),
                        PreparedPairValue::Literal(_) => None,
                    });
                    let mut realized = realize_integer_row(row.as_ref(), None, (0..roots.parameters).chain(selected), work)?;
                    let returned = points(&realized)?;
                    realized.truncate(roots.parameters);
                    found = Some(SymbolicResultPairWitness::Joint { spans, values: returned, entry: IntegerInputs(vec![realized]),
                        axes: roots.axes[..roots.parameters].to_vec() });
                    Some(true)
                };
                for row in &path.0 {
                    let stopped = match clause {
                        ResultPairClause::Joint { predicate, .. } => predicate.visit(row, &values, [true, false], work, &mut accept)?,
                        ResultPairClause::Literal(literal) => {
                            charge_call_work(work, 1)?;
                            match *values.get(literal.slot)? {
                                PreparedPairValue::Literal(value) => visit_pair_constant(row,
                                    literal.relation.accepts(value.cmp(&literal.bound)), [true, false], work, &mut accept)?,
                                PreparedPairValue::Axis(axis) => IntegerInputs::visit_ordinary_atom::<true>(row,
                                    &OrdinaryIntegerAtom::Literal(axis, literal.relation, &literal.bound),
                                    [true, false], work, &mut accept)?,
                            }
                        }
                    };
                    if stopped { break; }
                }
                if found.is_some() {
                    check.failure = Some(ResultPairFailure { step, source_slot: clause.slot(), witness: found });
                    break;
                }
            }
        }
        Some(())
    }
}

#[cfg(all(test, feature = "internal-checker"))]
#[test]
fn joint_result_proof_stops_before_budgeting_later_comparison_rows() {
    let left = IntegerSet::closed_interval(0u8.into(), 7u8.into()).unwrap();
    let right = IntegerSet::closed_interval(0u8.into(), 200u8.into()).unwrap();
    let inputs = IntegerInputs::from_cells(vec![vec![Rc::new(left), Rc::new(right)]]);
    let relation = IntegerRelation::from_operator(&BinaryOperator::NotEqual).unwrap();
    let mut exhaustive_work = 0;
    let _ = inputs.ordinary_comparison([0, 1], relation, [true, false], &mut exhaustive_work).unwrap();
    let mut work = MAX_PARTITION_TRANSFER_WORK - exhaustive_work + 1;
    let predicate = ResultPairBoolean::Compare {
        operands: [ResultPairTerm::Slot(0), ResultPairTerm::Slot(1)], relation,
    };
    let [row] = inputs.0.as_slice() else { panic!("one independent input row"); };
    let mut observed = 0;
    let stopped = predicate.visit(row, &[PreparedPairValue::Axis(0), PreparedPairValue::Axis(1)],
        [true, false], &mut work, &mut |rejected, lane, _| {
            observed += 1;
            assert_eq!(lane, 0);
            assert_eq!(rejected[0].singleton_value(), Some(&BigInt::ZERO));
            assert_eq!(rejected[1].singleton_value(), Some(&BigInt::ZERO));
            Some(true)
        });
    assert_eq!(stopped, Some(true));
    assert_eq!(observed, 1);
    assert!(work < MAX_PARTITION_TRANSFER_WORK);
}

#[cfg(all(test, feature = "internal-checker"))]
#[test]
fn joint_result_proof_exact_legacy_checks_do_not_charge_symbolic_row_work() {
    let equal = IntegerRelation::from_operator(&BinaryOperator::Equal).unwrap();
    let literal = |value: u8| ResultPairClause::Literal(ResultSlotLiteral { slot: 1, relation: equal, bound: value.into() });
    let span = super::ByteSpan::new(2, 3);
    let plan = ResultPairProof { terminal: literal(1), show: Some((span, literal(1))), same_literal: true };
    let mut work = MAX_PARTITION_TRANSFER_WORK;
    assert!(matches!(plan.exact_failure(&[3u8.into(), 1u8.into()], &mut work), Some(None)));
    let Some(Some(failed)) = plan.exact_failure(&[9u8.into(), 0u8.into()], &mut work) else { panic!("failed show"); };
    assert_eq!(failed.step, Some(span));
    assert_eq!(failed.source_slot, Some(1));
    assert!(failed.witness.is_none());
    assert_eq!(work, MAX_PARTITION_TRANSFER_WORK);
}

#[cfg(feature = "internal-checker")]
pub(super) fn collect_result_pair_proofs<'a>(
    calls: &'a FunctionCallSites, entry: ResultPairEntry<'_>, checks: &mut [ResultPairCheck<'_, 'a>], work: &mut usize,
) -> Option<(bool, Option<[BigInt; 2]>)> {
    let (exact, filter) = match entry {
        ResultPairEntry::Exact(arguments) => (Some(arguments), None),
        ResultPairEntry::Symbolic(filter) => (None, Some(filter)),
    };
    let mut legacy = [Vec::new(), Vec::new()];
    if exact.is_none() {
        for check in checks.iter().filter(|check| check.admitted) {
            if let Some((goal, show)) = check.proof.legacy() {
                for literal in show.into_iter().map(|(_, literal)| literal).chain(std::iter::once(goal)) {
                    legacy.get_mut(literal.slot)?.push(SymbolicLiteralCheck {
                        relation: literal.relation, bound: &literal.bound, failure: None,
                    });
                }
            }
        }
    }
    let observed = std::cell::RefCell::new(ResultPairState {
        checks, legacy, exact: exact.is_some(), returned: None, observed: false, work: *work,
    });
    #[cfg(test)]
    INTEGER_OBSERVATIONS.with(|count| { let (symbolic, actual) = count.get();
        count.set((symbolic + usize::from(exact.is_none()), actual + usize::from(exact.is_some()))); });
    let first = &calls.boundary.results.first()?.ty;
    let coverage = run_integer_callable::<false, false, _>(calls, Some(first), filter, None, &mut [],
        exact, None, None, false, ResultPairSink(&observed))?.ok()?;
    let state = observed.into_inner();
    *work = state.work;
    if !state.exact && state.observed {
        let mut results = state.legacy.map(Vec::into_iter);
        for check in state.checks.iter_mut().filter(|check| check.admitted) {
            if let Some((terminal, show)) = check.proof.legacy() {
                let shown = if let Some((step, show)) = show {
                    results.get_mut(show.slot)?.next()?.failure.map(|witness| ResultPairFailure {
                        step: Some(step), source_slot: Some(show.slot), witness: Some(SymbolicResultPairWitness::Literal(witness)),
                    })
                } else { None };
                let terminal_verdict = results.get_mut(terminal.slot)?.next()?.failure;
                check.failure = shown.or_else(|| terminal_verdict.map(|witness| ResultPairFailure {
                    step: None, source_slot: Some(terminal.slot), witness: Some(SymbolicResultPairWitness::Literal(witness)),
                }));
            }
        }
    }
    match coverage {
        ProofCoverage::Applicable(_) if state.observed => Some((true, state.returned)),
        ProofCoverage::NoApplicableInputs if exact.is_none() && !state.observed => Some((false, None)),
        _ => None,
    }
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
    observe_typed_scalar_returns(calls, tail, entry_filter, exact_arguments, proof_reflexive_branches, authored_return)
}

fn observe_typed_scalar_returns<'a, Authored: ScopedAuthoredReturn<'a>>(
    calls: &'a FunctionCallSites, tail: &'a ScalarType, entry_filter: Option<&[CheckerPredicate]>,
    exact_arguments: Option<&[&BigInt]>, proof_reflexive_branches: bool, authored_return: Authored,
) -> Option<Result<ProofCoverage<Vec<IntegerReturningPath<'a>>>, IntegerCheckFailure<'a>>> {
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

#[cfg(feature = "internal-checker")]
pub(super) fn pure_affine_proof_body(calls: &FunctionCallSites) -> bool {
    let structure = &calls.structure;
    // Charge repeated producer activations through a fixed admitted body
    // size as well as the existing per-path comparison/row work meters.
    const MAX_PURE_PROOF_OCCURRENCES: usize = 256;
    structure.checker_has_affine_scalar_local && !structure.checker_has_finite_scalar_local
        && calls.occurrences.len() <= MAX_PURE_PROOF_OCCURRENCES && structure.checker_directives.is_empty()
        && !structure.has_ordinary_contextual_places() && !structure.checker_has_bool_contextual_declaration
        && structure.checker_selected_snapshots.is_empty()
        && calls.call_occurrences.is_empty() && calls.allocation_length_calls.is_empty() && calls.assignments.is_empty()
        && matches!(calls.boundary.parameters.as_slice(), [parameter]
            if integer_tail_type(calls) == Some(&parameter.ty))
}

#[cfg(feature = "internal-checker")]
#[derive(Clone, Copy)]
struct PureAffineReturn;

#[cfg(feature = "internal-checker")]
impl<'a> ScopedAuthoredReturn<'a> for PureAffineReturn {
    const ENABLED: bool = false;
    const ALLOW_ORDINARY_INTEGER_LOCALS: bool = true;
    const REQUIRE_TOTAL_RETURN: bool = true;

    fn allows_extended_scalar_locals(self) -> bool { true }

    fn at(
        self, _: &FunctionCallSites, _: BlockOrdinal, _: FinalOutputSlot, _: &ScalarType,
        _: &IntegerRoots<'_>, _: &IntegerInputs,
    ) -> Option<&'a BigInt> {
        None
    }
}

pub(super) fn exact_integer_return_at_entry(
    calls: &FunctionCallSites,
    expected: &ScalarType,
    arguments: &[&BigInt],
) -> Option<BigInt> {
    exact_integer_return_at_entry_mode(calls, expected, arguments, ())
}

#[cfg(feature = "internal-checker")]
pub(super) fn exact_named_pure_affine_return_at_entry(
    calls: &FunctionCallSites, expected: &ScalarType, arguments: &[&BigInt],
) -> Option<BigInt> {
    exact_integer_return_at_entry_mode(calls, expected, arguments, PureAffineReturn)
}

#[cfg(feature = "internal-checker")]
pub(super) fn exact_named_literal_return_at_entry(
    calls: &FunctionCallSites, expected: &ScalarType, arguments: &[&BigInt],
) -> Option<BigInt> {
    let tail = closed_literal_tail_type(calls, expected)?;
    exact_typed_leaf_return_at_entry(calls, tail, arguments, PureAffineReturn)
}

fn exact_integer_return_at_entry_mode<'a, Authored: ScopedAuthoredReturn<'a>>(
    calls: &'a FunctionCallSites, expected: &ScalarType, arguments: &[&BigInt], mode: Authored,
) -> Option<BigInt> {
    let tail = integer_tail_type(calls).filter(|tail| *tail == expected)?;
    exact_typed_leaf_return_at_entry(calls, tail, arguments, mode)
}

fn exact_typed_leaf_return_at_entry<'a, Authored: ScopedAuthoredReturn<'a>>(
    calls: &'a FunctionCallSites, tail: &'a ScalarType, arguments: &[&BigInt], mode: Authored,
) -> Option<BigInt> {
    let ProofCoverage::Applicable(paths) = observe_typed_scalar_returns(calls, tail, None, Some(arguments), false, mode)?.ok()?
        else { return None; };
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


#[cfg(feature = "internal-checker")]
fn closed_literal_tail_type<'a>(calls: &'a FunctionCallSites, expected: &ScalarType) -> Option<&'a ScalarType> {
    if *expected == ScalarType::Char {
        let ([result], [output]) = (calls.boundary.results.as_slice(), calls.boundary.root_outputs.as_slice())
            else { return None; };
        return (result.ty == ScalarType::Char && output.ty == result.ty).then_some(&result.ty);
    }
    ordinary_tail_type(calls, *expected == ScalarType::Bool).filter(|tail| *tail == expected)
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
        allow_extended_scalar_locals: authored_return.allows_extended_scalar_locals(),
        #[cfg(feature = "internal-checker")]
        allow_symbolic_contextual_bool: COLLECT && !USE_IMPORTED_BOUND
            && authored_return.allows_extended_scalar_locals() && calls.structure.checker_has_bool_contextual_declaration,
        #[cfg(feature = "internal-checker")]
        allow_ordinary_affine_overwrites: Authored::ALLOW_ORDINARY_INTEGER_LOCALS || authored_return.allows_snapshot_history(),
        #[cfg(feature = "internal-checker")]
        allow_snapshot_history: authored_return.allows_snapshot_history(),
        #[cfg(feature = "internal-checker")]
        allow_ordinary_contextual_places: (Authored::ALLOW_ORDINARY_INTEGER_LOCALS || authored_return.allows_context_history())
            && calls.structure.has_ordinary_contextual_places(),
        #[cfg(feature = "internal-checker")]
        ordinary_comparison_work: ((Authored::ALLOW_ORDINARY_INTEGER_LOCALS
            || authored_return.allows_imported_condition_comparisons())
            && calls.structure.has_ordinary_predicates()).then_some(0),
        #[cfg(feature = "internal-checker")]
        symbolic_context_work: Authored::OBSERVE_FIXED_ARRAY.then_some(0),
        #[cfg(feature = "internal-checker")]
        callable_history: authored_return.callable_history(),
        #[cfg(feature = "internal-checker")]
        startup_trace: authored_return.startup_trace(),
        ..IntegerRoots::default()
    };
    for (binding, parameter) in checker_parameters.iter().zip(&boundary.parameters) {
        roots.add(binding, &parameter.ty);
    }
    let projected = if COLLECT { failures.as_deref_mut()?.projected.take() } else { None };
    let projected_slot = projected.as_ref().map(|(slot, _)| *slot);
    #[cfg(feature = "internal-checker")]
    let fixed_array_entries = Authored::OBSERVE_FIXED_ARRAY.then(|| authored_return.take_fixed_array_entries()).flatten();
    #[cfg(not(feature = "internal-checker"))]
    let fixed_array_entries: Option<(usize, IntegerInputs)> = None;
    let entry = if let Some((arity, entries)) = fixed_array_entries {
        if entry_filter.is_some() || exact_arguments.is_some() || projected.is_some() || arity != roots.parameters {
            return None;
        }
        if entries.is_unreachable() { return Some(Ok(ProofCoverage::NoApplicableInputs)); }
        entries
    } else {
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
            let allow_bool = Authored::ALLOW_ORDINARY_INTEGER_LOCALS
                || (roots.callable_history.is_some() && calls.checker_has_authored_match_history());
            #[cfg(not(feature = "internal-checker"))]
            let allow_bool = false;
            boundary.parameters.iter()
                .map(|parameter| {
                    #[cfg(feature = "internal-checker")]
                    if tail == Some(&ScalarType::Char) && parameter.ty == ScalarType::Char {
                        return Some(Rc::new(IntegerSet::ordinary_character_domain()));
                    }
                    ordinary_parameter_set(&parameter.ty, allow_bool).map(Rc::new)
                })
                .collect::<Option<IntegerRow>>()?
        };
        for filter in entry_filter.into_iter().flatten() {
            match filter {
                CheckerPredicate::Boolean(true) => {}
                CheckerPredicate::Boolean(false) => return Some(Ok(ProofCoverage::NoApplicableInputs)),
                CheckerPredicate::NamedBoolEntry { .. } | CheckerPredicate::DirectiveDisjunction { .. }
                | CheckerPredicate::ProofReflexiveBranch(_) => return None,
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
        IntegerInputs(vec![row])
    };
    let result = walk_integer_block::<USE_IMPORTED_BOUND, COLLECT, _>(calls, &mut roots, calls.structure.root, entry, tail, returns, &mut handle_call, &mut failures, &mut imports, authored_return)?;
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
    #[cfg(feature = "internal-checker")]
    let mut retained_unit_paths = false;
    #[cfg(not(feature = "internal-checker"))]
    let retained_unit_paths = false;
    #[cfg(feature = "internal-checker")]
    if let Some(tracker) = roots.callable_history {
        let mut tracker = tracker.borrow_mut();
        if !tracker.stopped {
            if let Some(arity) = tracker.unit_return_arity {
                retained_unit_paths = true;
                let IntegerControl::Continue(path) = &result else { return None; };
                tracker.charge(path.0.len())?;
                let mut pieces = Vec::with_capacity(path.0.len());
                for row in &path.0 {
                    let domains = row.cells.get(..arity)?;
                    let cost = domains.iter().try_fold(arity, |cost, domain| cost.checked_add(domain.interval_count()))?;
                    tracker.charge(cost)?;
                    pieces.push(UnitHistoryPiece { domains: domains.to_vec(), steps: tracker.steps(row.callable_history)? });
                }
                tracker.unit_return_paths = Some(pieces);
            }
        }
    }
    if COLLECT {
        failures.as_deref_mut()?.continuation = match &result {
            IntegerControl::Continue(path) if path.is_unreachable() => IntegerRequirementContinuation::Empty,
            IntegerControl::Continue(_) if retained_unit_paths => IntegerRequirementContinuation::Feasible,
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
    if comparison.contextual && !roots.allow_exact_contextual_resource_guard { return None; }
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
            Self::visit_ordinary_atom::<false>(row, &atom, selected, work, &mut |retained, lane, work| {
                if matches!(retained, Cow::Borrowed(_)) {
                    charge_call_work(work, retained.len().checked_add(1)?)?;
                }
                output[lane].0.push(retained.into_owned());
                Some(false)
            })?;
        }
        Some(output)
    }

    // true from the visitor stops the search. A constant cut exposes its
    // original row without copying; materialized bounded cuts are shared with
    // ordinary branching (which continues until both requested lanes close).
    fn visit_ordinary_atom<const STOP_EARLY: bool>(
        row: &IntegerRow, atom: &OrdinaryIntegerAtom<'_>, selected: [bool; 2], work: &mut usize,
        accept: &mut impl FnMut(Cow<'_, IntegerRow>, usize, &mut usize) -> Option<bool>,
    ) -> Option<bool> {
        let cut = match atom {
            OrdinaryIntegerAtom::Lexical(axes, relation) => {
                let lookups = usize::BITS - row.equal_axes().len().leading_zeros();
                charge_call_work(work, 8 + 2 * lookups as usize)?;
                ordinary_comparison_cut(row, *axes, *relation)?
            }
            OrdinaryIntegerAtom::Literal(axis, relation, bound) => {
                charge_call_work(work, 1)?;
                OrdinaryComparisonCut::Bound { axis: *axis, relation: *relation, bound: Cow::Borrowed(bound) }
            }
        };
        match cut {
            OrdinaryComparisonCut::Constant(result) =>
                if selected[result as usize] { accept(Cow::Borrowed(row), result as usize, work) } else { Some(false) },
            OrdinaryComparisonCut::Bound { axis, relation, bound } =>
                Self::visit_ordinary_bound(row, axis, relation, &bound, None, selected, work, accept),
            OrdinaryComparisonCut::Independent { axes, relation } => {
                if let Some(uniform) = independent_uniform(&row[axes[0]], &row[axes[1]], relation, work)? {
                    if selected[uniform as usize] { accept(Cow::Borrowed(row), uniform as usize, work) } else { Some(false) }
                } else {
                    Self::visit_independent_comparison::<STOP_EARLY>(row, axes, relation, selected, work, accept)
                }
            }
        }
    }

    fn visit_independent_comparison<const STOP_EARLY: bool>(
        row: &IntegerRow, [left, right]: [usize; 2], relation: IntegerRelation, selected: [bool; 2], work: &mut usize,
        accept: &mut impl FnMut(Cow<'_, IntegerRow>, usize, &mut usize) -> Option<bool>,
    ) -> Option<bool> {
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
        if !STOP_EARLY { charge_call_work(work, count.checked_add(row[small].interval_count())?)?; }
        for (first, last) in row[small].intervals() {
            if STOP_EARLY { charge_call_work(work, 1)?; }
            let mut point = first.clone();
            loop {
                if STOP_EARLY { charge_call_work(work, 1)?; }
                if Self::visit_ordinary_bound(row, broad, relation, &point, Some(small), selected, work, accept)? {
                    return Some(true);
                }
                if &point == last { break; }
                point += 1u8;
            }
        }
        Some(false)
    }

    fn visit_ordinary_bound(
        row: &IntegerRow, axis: usize, relation: IntegerRelation, bound: &BigInt, fixed: Option<usize>,
        selected: [bool; 2], work: &mut usize,
        accept: &mut impl FnMut(Cow<'_, IntegerRow>, usize, &mut usize) -> Option<bool>,
    ) -> Option<bool> {
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
            if accept(Cow::Owned(retained), lane, work)? { return Some(true); }
        }
        Some(false)
    }

    fn ordinary_boolean(
        &self, condition: &super::reference::CheckerOrdinaryBoolean, roots: &IntegerRoots<'_>,
        selected: [bool; 2], work: &mut usize,
        history: Option<(&FunctionCallSites, OccurrenceOrdinal, &std::cell::RefCell<CallableHistories<'_>>)>,
    ) -> Option<[Self; 2]> {
        use super::reference::CheckerOrdinaryBoolean as Boolean;
        if history.is_some_and(|(calls, site, _)|
            calls.structure.checked_types.expressions.get(&site) != Some(&ScalarType::Bool))
        { return None; }
        let child_history = |edge| -> Option<_> {
            let Some((calls, ordinal, history)) = history else { return Some(None); };
            let OccurrenceNode::Expression(child) = calls.structure.edges.get(&(OccurrenceParent::Expression(ordinal), edge))?
                else { return None; };
            Some(Some((calls, *child, history)))
        };
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
            Boolean::Binding(binding) | Boolean::ContextualBinding(binding) => {
                if matches!(condition, Boolean::ContextualBinding(_))
                    && !roots.allow_exact_contextual_bool && !roots.allow_symbolic_contextual_bool
                {
                    return None;
                }
                let (axis, ty) = roots.lookup(binding)?;
                if *ty != ScalarType::Bool { return None; }
                let one = BigInt::from(1u8);
                let relation = IntegerRelation::from_operator(&BinaryOperator::Equal)?;
                output = self.ordinary_atom(OrdinaryIntegerAtom::Literal(axis, relation, &one), selected, work)?;
            }
            Boolean::Integer(predicate) => {
                let (axis, ty) = roots.lookup(&predicate.binding)?;
                if ty != &predicate.ty { return None; }
                let relation = IntegerRelation::from_operator(&predicate.operator)?;
                output = self.ordinary_atom(OrdinaryIntegerAtom::Literal(axis, relation, &predicate.bound), selected, work)?;
            }
            Boolean::Lexical(comparison) => {
                let axes = prepared_comparison_axes(roots, comparison)?;
                output = self.ordinary_comparison(axes, comparison.relation, selected, work)?;
            }
            Boolean::Not(value) => {
                let [no, yes] = self.ordinary_boolean(value, roots, [selected[1], selected[0]], work,
                    child_history(OccurrenceEdge::UnaryOperand)?)?;
                return Some([yes, no]);
            }
            Boolean::And(operands) | Boolean::Or(operands) => {
                let and = matches!(condition, Boolean::And(_));
                let continuing = usize::from(and);
                let stop = 1 - continuing;
                let mut left_selected = selected;
                left_selected[continuing] = true;
                let mut left = self.ordinary_boolean(&operands[0], roots, left_selected, work,
                    child_history(OccurrenceEdge::BinaryLeft)?)?;
                let right = left[continuing].ordinary_boolean(&operands[1], roots, selected, work,
                    child_history(OccurrenceEdge::BinaryRight)?)?;
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
        // Only evaluated atoms belong to the path; the right operand
        // of an already-decided short circuit is never visited.
        if !matches!(condition, Boolean::And(_) | Boolean::Or(_)) {
            if let Some((calls, ordinal, tracker)) = history {
                let span = calls.occurrences.get(ordinal.0)?.span;
                let mut tracker = tracker.borrow_mut();
                for (lane, rows) in output.iter_mut().enumerate() {
                    charge_call_work(work, rows.0.len())?;
                    let decision = lane != 0;
                    let step = tracker.step_owner.map_or(CallableHistoryStep::Branch(span, decision),
                        |owner| CallableHistoryStep::ProviderBranch(owner, span, decision));
                    for row in &mut rows.0 { tracker.append(row, step); }
                }
            }
        }
        Some(output)
    }

    fn callable_compound_branch(
        &self, calls: &FunctionCallSites, condition: OccurrenceOrdinal, roots: &IntegerRoots<'_>,
    ) -> Option<Option<[Self; 2]>> {
        let Some(tracker) = roots.callable_history else { return Some(None); };
        let Some(tree) = calls.structure.checker_ordinary_booleans.get(&condition) else { return Some(None); };
        if !call_free_history_boolean(tree) { return Some(None); }
        let mut work = tracker.borrow().work.get();
        let rows = self.ordinary_boolean(tree, roots, [true, true], &mut work, Some((calls, condition, tracker)))?;
        tracker.borrow().work.set(work);
        Some(Some(rows))
    }

    fn ordinary_predicate_at(
        &self, structure: &OccurrenceStructure, condition: OccurrenceOrdinal, roots: &mut IntegerRoots<'_>,
        selected: [bool; 2],
    ) -> Option<Option<[Self; 2]>> {
        if roots.named_exact_bool {
            use super::reference::CheckerOrdinaryBoolean as Boolean;
            let (binding, negated) = match structure.checker_ordinary_booleans.get(&condition) {
                Some(Boolean::Binding(binding)) => (binding, false),
                Some(Boolean::Not(operand)) => match operand.as_ref() {
                    Boolean::Binding(binding) => (binding, true),
                    _ => return Some(None),
                },
                _ => return Some(None),
            };
            let (axis, ty) = roots.lookup(binding)?;
            if *ty != ScalarType::Bool || axis >= roots.parameters { return None; }
            let relation = IntegerRelation::from_operator(if negated { &BinaryOperator::NotEqual } else { &BinaryOperator::Equal })?;
            let work = roots.ordinary_comparison_work.get_or_insert(0);
            return self.ordinary_atom(OrdinaryIntegerAtom::Literal(axis, relation, &BigInt::from(1u8)), selected, work).map(Some);
        }
        let Some(mut work) = roots.ordinary_comparison_work else { return Some(None); };
        let partitions = if let Some(compound) = structure.checker_ordinary_booleans.get(&condition) {
            self.ordinary_boolean(compound, roots, selected, &mut work, None)?
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
fn group_callable_rows<T>(
    rows: impl IntoIterator<Item = T>, history: impl Fn(&T) -> usize,
    count: usize, tracker: &std::cell::RefCell<CallableHistories<'_>>,
) -> Option<Vec<Vec<T>>> {
    tracker.borrow().charge(count.checked_mul(2)?)?;
    let mut order = HashMap::new();
    let mut groups = Vec::<Vec<T>>::new();
    for row in rows {
        let group = *order.entry(history(&row)).or_insert_with(|| {
            let index = groups.len();
            groups.push(Vec::new());
            index
        });
        groups[group].push(row);
    }
    Some(groups)
}

// Use the same typed numeric partition as the ordinary Boolean walker,
// but keep the two authored operand decisions lost by its compact union.
#[cfg(feature = "internal-checker")]
fn partition_snapshot_disjunction(
    roots: &IntegerRoots<'_>, before: &IntegerInputs, predicate: &CheckerPredicate, work: &mut usize,
    mut event: impl FnMut(&mut IntegerInputs, super::ByteSpan, bool, &mut usize) -> Option<()>,
) -> Option<[IntegerInputs; 2]> {
    let CheckerPredicate::DirectiveDisjunction {
        binding, ty, atoms: [(left, first), (right, second)], operands: [lhs, rhs],
    } = predicate else { return None; };
    let (axis, checked) = roots.lookup(binding)?;
    if ty != checked { return None; }
    let [mut remainder, mut first_pass] = before.ordinary_atom(
        OrdinaryIntegerAtom::Literal(axis, *left, first), [true, true], work)?;
    event(&mut first_pass, *lhs, true, work)?;
    event(&mut remainder, *lhs, false, work)?;
    let [mut rejecting, mut second_pass] = remainder.ordinary_atom(
        OrdinaryIntegerAtom::Literal(axis, *right, second), [true, true], work)?;
    event(&mut second_pass, *rhs, true, work)?;
    event(&mut rejecting, *rhs, false, work)?;
    charge_call_work(work, first_pass.0.len().checked_add(second_pass.0.len())?)?;
    first_pass.0.append(&mut second_pass.0);
    Some([rejecting, first_pass])
}

#[cfg(feature = "internal-checker")]
fn project_callable_compound_directive<'a>(
    calls: &'a FunctionCallSites, roots: &mut IntegerRoots<'a>, path: &mut IntegerInputs,
    site: OccurrenceOrdinal, tree: Option<&super::reference::CheckerOrdinaryBoolean>,
    operation: CheckerOperation, failures: &mut Option<&mut IntegerFailureCollection<'a, '_>>,
) -> Option<()> {
    let tracker = roots.callable_history?;
    let require = operation == CheckerOperation::Require;
    // Group the pre-directive structural histories before evaluating any
    // predicate. A Narrow can fail only when *all* its group's rows fail.
    let mut survivors = IntegerInputs::default();
    for group in group_callable_rows(&path.0, |row| row.callable_history, path.0.len(), tracker)? {
        let mut work = tracker.borrow().work.get();
        let before = IntegerInputs(group.into_iter().map(|row| {
            charge_call_work(&mut work, row.len().checked_add(1)?)?;
            Some(row.clone())
        }).collect::<Option<Vec<_>>>()?);
        let [rejecting, passing] = if let Some(tree) = tree {
            let OccurrenceNode::Expression(condition) = calls.structure.edges.get(&(
                OccurrenceParent::Expression(site), OccurrenceEdge::CallArgument(0),
            ))? else { return None; };
            before.ordinary_boolean(tree, roots, [require, !require],
                &mut work, Some((calls, *condition, tracker)))?
        } else {
            if require || !roots.allow_snapshot_history { return None; }
            let predicate = calls.structure.checker_directives.get(&site)?.predicate.as_ref()?;
            // The existing shared history meter also covers each retained
            // original operand event; no other domain operation intervenes.
            let tracker_work = tracker.borrow().work;
            let [rejecting, passing] = partition_snapshot_disjunction(roots, &before, predicate, &mut work,
                |lane, span, yes, shared| {
                    tracker_work.set(*shared);
                    tracker.borrow_mut().append_rows(lane, span, yes)?;
                    *shared = tracker_work.get();
                    Some(())
                })?;
            [rejecting, passing]
        };
        tracker.borrow().work.set(work);
        let rejected = if require {
            // The assertion must not affect subsequent flow. Only its
            // selected failure packets carry the evaluated operand paths.
            let count = rejecting.0.len();
            group_callable_rows(rejecting.0, |row| row.callable_history, count, tracker)?
        } else if passing.is_unreachable() {
            // Report the authentic pre-Narrow witness exactly once, without
            // promoting hypothetical rejection candidates to continuations.
            vec![before.0]
        } else {
            tracker.borrow().charge(passing.0.len())?;
            survivors.0.extend(passing.0);
            Vec::new()
        };
        for rows in rejected {
            let witness = IntegerInputs(rows);
            let mut work = tracker.borrow().work.get();
            charge_call_work(&mut work, roots.axes.len().checked_add(1)?)?;
            let equal_axes = witness.common_equal_axes_bounded(&mut work)?;
            tracker.borrow().work.set(work);
            (failures.as_deref_mut()?.sink)(IntegerCheckFailure {
                directive: site, state: witness, axes: roots.axes.clone(), equal_axes,
            });
            roots.after_startup_candidate()?;
        }
    }
    if !require { *path = survivors; }
    roots.reached_obligation = true;
    Some(())
}

#[cfg(feature = "internal-checker")]
pub(super) fn call_free_history_boolean(condition: &super::reference::CheckerOrdinaryBoolean) -> bool {
    use super::reference::CheckerOrdinaryBoolean as Boolean;
    fn valid(node: &Boolean) -> bool {
        match node {
            Boolean::And(children) | Boolean::Or(children) => children.iter().all(valid),
            Boolean::Not(inner) => valid(inner),
            Boolean::Constant(_) | Boolean::Integer(_) | Boolean::Lexical(_) => true,
            _ => false,
        }
    }
    matches!(condition, Boolean::And(_) | Boolean::Or(_)) && valid(condition)
}

#[cfg(feature = "internal-checker")]
struct OrdinaryGuard {
    condition: OccurrenceOrdinal,
    next: Option<super::reference::CheckerOrdinaryBoolean>,
    frames: Vec<GuardFrame>,
    returned: Option<[IntegerInputs; 2]>,
    directive_before: Option<IntegerInputs>,
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
                            self.returned = Some(path.ordinary_boolean(&leaf, roots, [true, true], &mut work, None)?);
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

fn plain_typed_return<'a>(
    calls: &'a FunctionCallSites, slot: &FinalOutputSlot, ty: &ScalarType, roots: &IntegerRoots<'a>,
) -> Option<IntegerReturn<'a>> {
    if let Some(value) = scalar_call_literal(&calls.structure, slot.expression, ty, false) {
        return Some(IntegerReturn::Const(value));
    }
    let PlainNameIdentity::Lexical(binding) = calls.prepared_plain_name(slot.expression)? else { return None; };
    let (_, binding_type) = roots.lookup(binding)?;
    (binding_type == ty).then_some(IntegerReturn::Alias(binding))
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
        #[cfg(feature = "internal-checker")]
        if let Some(work) = &mut roots.symbolic_context_work {
            charge_partition_capture(work, &path, roots.axes.len())?;
        }
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
        let region = structure.blocks.get(block.0)?;
        #[cfg(feature = "internal-checker")]
        if (Authored::OBSERVE_RUNTIME_FIXED_INDEX || roots.symbolic_context_entry_amount) && region.unsafe_context { return None; }
        let items = &region.items;
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
        // A closed contextual owner may retain a required proof observation
        // after the authored search has stopped. A Require cannot change the
        // feasible typed row; a Narrow still must prune it before the call.
        #[cfg(feature = "internal-checker")]
        if roots.startup_trace.as_ref().is_some_and(|(_, trace)| trace.borrow().proof_only_continuation()) {
            if let BlockItemKind::Evaluation { expression } = item.kind {
                if let Some(directive) = structure.checker_directives.get(&expression) {
                    let predicate = directive.predicate.as_ref()?;
                    if !matches!(predicate, CheckerPredicate::Boolean(_) | CheckerPredicate::Integer(_)) { return None; }
                    if directive.operation == CheckerOperation::Narrow { path = path.split(roots, predicate)?.0; }
                    continue;
                }
            }
        }
        #[cfg(feature = "internal-checker")]
        let mut evaluated_guard = {
            let mut result = (None, None);
            if SUSPEND_CALL && suspend_integer && roots.ordinary_comparison_work.is_some()
                && (!structure.checker_ordinary_call_guards.is_empty() || !structure.checker_ordinary_directive_calls.is_empty())
            {
                let condition = match &item.kind {
                    BlockItemKind::Evaluation { expression } if structure.checker_ordinary_directive_calls.contains_key(expression) =>
                        Some(*expression),
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
                    structure.checker_ordinary_call_guards.get(&condition).copied()
                        .or_else(|| structure.checker_ordinary_directive_calls.contains_key(&condition).then_some(1))
                        .map(|nodes| (condition, nodes)))
                {
                    if guard.is_none() {
                        let work = roots.ordinary_comparison_work.as_mut()?;
                        charge_call_work(work, nodes)?;
                        let (boolean, directive_before) = if structure.checker_ordinary_directive_calls.contains_key(&condition) {
                            charge_partition_capture(work, &path, roots.axes.len())?;
                            (super::reference::CheckerOrdinaryBoolean::Call(condition), Some(IntegerInputs(path.0.clone())))
                        } else {
                            (structure.checker_ordinary_booleans.get(&condition)?.clone(), None)
                        };
                        guard = Some(OrdinaryGuard {
                            condition, next: Some(boolean), frames: Vec::new(), returned: None, directive_before,
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
                        GuardProgress::Done(rows) => {
                            result = (Some(rows), active.directive_before.take());
                            guard = None;
                        }
                    }
                } else if guard.is_some() { return None; }
            }
            result
        };
        match item.kind {
            BlockItemKind::Evaluation { expression } => {
                if let Some(directive) = structure.checker_directives.get(&expression) {
                    #[cfg(feature = "internal-checker")]
                    if COLLECT && roots.allow_snapshot_history && directive.operation == CheckerOperation::Narrow
                        && matches!(directive.predicate, Some(CheckerPredicate::DirectiveDisjunction { .. }))
                    {
                        let predicate = directive.predicate.as_ref()?;
                        if roots.callable_history.is_some() {
                            project_callable_compound_directive(calls, roots, &mut path, expression, None, directive.operation, failures)?;
                            continue;
                        }
                        if let Some((source, trace)) = roots.startup_trace.as_ref() {
                            let [row] = path.0.as_slice() else { return None; };
                            if row.iter().any(|domain| domain.singleton_value().is_none()) { return None; }
                            let work = roots.ordinary_comparison_work.get_or_insert(0);
                            let mut budget = *work;
                            let mut events = Vec::with_capacity(2);
                            let [rejecting, passing] = partition_snapshot_disjunction(roots, &path, predicate, &mut budget,
                                |lane, span, taken, shared| {
                                    if !lane.is_unreachable() {
                                        charge_call_work(shared, 1)?;
                                        events.push((span, taken));
                                    }
                                    Some(())
                                })?;
                            roots.ordinary_comparison_work = Some(budget);
                            roots.reached_obligation = true;
                            if passing.is_unreachable() == rejecting.is_unreachable() { return None; }
                            if passing.is_unreachable() {
                                let witness = std::mem::take(&mut path);
                                (failures.as_deref_mut()?.sink)(IntegerCheckFailure {
                                    directive: expression, equal_axes: witness.common_equal_axes(), state: witness, axes: roots.axes.clone(),
                                });
                                roots.after_startup_candidate()?;
                            } else {
                                for (span, taken) in events {
                                    trace.borrow_mut().branch(wosy_syntax::SourceSpan::new((*source).clone(), span), taken)?;
                                }
                                path = passing;
                            }
                            continue;
                        }
                    }
                    #[cfg(feature = "internal-checker")]
                    if roots.startup_trace.is_some() && (evaluated_guard.1.is_some()
                        || !matches!(directive.predicate, Some(CheckerPredicate::Boolean(_) | CheckerPredicate::Integer(_))))
                    { return None; }
                    #[cfg(feature = "internal-checker")]
                    if evaluated_guard.1.is_some() {
                        let [rejected, accepted] = evaluated_guard.0.take()?;
                        let before = evaluated_guard.1.take()?;
                        if !COLLECT { return None; }
                        roots.reached_obligation = true;
                        let failing = if directive.operation == CheckerOperation::Require {
                            path = before;
                            rejected
                        } else if accepted.is_unreachable() {
                            before
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
                            #[cfg(feature = "internal-checker")]
                            roots.after_startup_candidate()?;
                        }
                        continue;
                    }
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
                                #[cfg(feature = "internal-checker")]
                                roots.after_startup_candidate()?;
                            }
                            continue;
                        }
                    }
                    #[cfg(feature = "internal-checker")]
                    if COLLECT && roots.callable_history.is_some() {
                        if let Some(tree) = structure.checker_ordinary_booleans.get(&expression).filter(|tree|
                            call_free_history_boolean(tree))
                        {
                            project_callable_compound_directive(calls, roots, &mut path, expression, Some(tree), directive.operation, failures)?;
                            continue;
                        }
                    }
                    let predicate = directive.predicate.as_ref()?;
                    #[cfg(feature = "internal-checker")]
                    if roots.projected_narrow_integer {
                        path.charge_projected_integer_directive(roots, predicate)?;
                    }
                    roots.reached_obligation = true;
                    #[cfg(feature = "internal-checker")]
                    if matches!(predicate, CheckerPredicate::Integer(_) | CheckerPredicate::DirectiveDisjunction { .. }) {
                        path.charge_local_alias_work(&mut roots.ordinary_local_work, 2)?;
                    }
                    if let (true, CheckerPredicate::DirectiveDisjunction { binding, ty, atoms, .. }) = (COLLECT, predicate) {
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
                            #[cfg(feature = "internal-checker")]
                            roots.after_startup_candidate()?;
                        }
                        continue;
                    }
                    #[cfg(feature = "internal-checker")]
                    if let Some(histories) = roots.callable_history {
                        if !COLLECT { return None; }
                        let axis = match predicate {
                            CheckerPredicate::Boolean(_) => None,
                            CheckerPredicate::Integer(predicate) => Some(roots.lookup(&predicate.binding)?.0),
                            _ => return None,
                        };
                        let mut emit = |rejecting: IntegerInputs| {
                            let tracker = histories.borrow();
                            tracker.charge(roots.axes.len().checked_add(1)?)?;
                            let mut work = tracker.work.get();
                            let equal_axes = rejecting.common_equal_axes_bounded(&mut work)?;
                            tracker.work.set(work);
                            drop(tracker);
                            (failures.as_deref_mut()?.sink)(IntegerCheckFailure {
                                directive: expression, equal_axes,
                                state: rejecting, axes: roots.axes.clone(),
                            });
                            roots.after_startup_candidate()
                        };
                        let charge_row = |row: &IntegerRow| {
                            let cuts = axis.and_then(|axis| row.get(axis)).map_or(0, |set| set.intervals().len());
                            histories.borrow().charge(
                                row.len().checked_add(cuts.checked_mul(2)?)?.checked_add(1)?)
                        };
                        if directive.operation == CheckerOperation::Require {
                            // Index already materialized rows in encounter
                            // order; never project a later path after cutoff.
                            for group in group_callable_rows(&path.0, |row| row.callable_history, path.0.len(), histories)? {
                                let mut rejecting = IntegerInputs::default();
                                for row in group {
                                    charge_row(row)?;
                                    let candidate = IntegerInputs(vec![row.clone()]);
                                    rejecting.0.extend(candidate.failing_witness(roots, predicate)?.0);
                                }
                                if !rejecting.is_unreachable() { emit(rejecting)?; }
                            }
                        } else {
                            // A Narrow is refuted only if none of the rows of
                            // the same structural history can satisfy it.
                            // Index groups in encounter order, not hash order.
                            histories.borrow().charge(path.0.len())?;
                            let mut order = HashMap::new();
                            let mut groups = Vec::<IntegerInputs>::new();
                            for row in std::mem::take(&mut path.0) {
                                let group = *order.entry(row.callable_history).or_insert_with(|| {
                                    let index = groups.len();
                                    groups.push(IntegerInputs::default());
                                    index
                                });
                                groups[group].0.push(row);
                            }
                            for group in groups {
                                for row in &group.0 { charge_row(row)?; }
                                let (passing, failing, _) = group.split(roots, predicate)?;
                                if passing.is_unreachable() {
                                    if !failing.is_unreachable() { emit(failing)?; }
                                } else {
                                    histories.borrow().charge(passing.0.len())?;
                                    path.0.extend(passing.0);
                                }
                            }
                        }
                        continue;
                    }
                    if COLLECT && directive.operation == CheckerOperation::Require {
                        let failing = path.failing_witness(roots, predicate)?;
                        if !failing.is_unreachable() {
                            (failures.as_deref_mut()?.sink)(IntegerCheckFailure {
                                directive: expression, equal_axes: failing.common_equal_axes(), state: failing, axes: roots.axes.clone(),
                            });
                            #[cfg(feature = "internal-checker")]
                            roots.after_startup_candidate()?;
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
                            #[cfg(feature = "internal-checker")]
                            roots.after_startup_candidate()?;
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
                    #[cfg(feature = "internal-checker")]
                    if let Some(histories) = roots.callable_history {
                        if !COLLECT || signature.parameters.len() > 2 { return None; }
                        let span = calls.occurrences.get(expression.0)?.span;
                        let mut survivors = IntegerInputs::default();
                        for mut row in std::mem::take(&mut path.0) {
                            let mut tracker = histories.borrow_mut();
                            tracker.charge(row.len().checked_add(3)?)?;
                            let call = tracker.boundary(span, false);
                            tracker.append(&mut row, call);
                            tracker.outgoing_call = Some(tracker.steps(row.callable_history)?);
                            drop(tracker);
                            let input = IntegerInputs(vec![row]);
                            let argument = if signature.parameters.len() == 2 {
                                nested_pair_argument(calls, expression, roots, &input, true, false, true, false, None)?
                            } else {
                                actual_nested_argument(calls, expression, roots, &input, true, false, None)
                            };
                            let operands = argument.callable_history_operands()?;
                            let domains_count = operands.iter().flatten().count();
                            let is_projected = operands.iter().flatten().any(|arg| matches!(arg, CallableHistoryOperand::Axis(..)));
                            if !matches!(handle(calls, expression, signature, argument, &ScalarType::Unit)?, ActualCallResult::Unit) {
                                return None;
                            }
                            let mut tracker = histories.borrow_mut();
                            let pieces = tracker.received_unit_paths.take()?;
                            if !is_projected && pieces.len() > 1 { return None; }
                            let returned = tracker.boundary(span, true);
                            for piece in pieces {
                                let original = input.0.first()?;
                                if piece.domains.len() != domains_count { return None; }
                                tracker.charge(original.len().checked_add(piece.steps.len())?.checked_add(1)?)?;
                                let mut next = original.clone();
                                for (operand, domain) in operands.into_iter().flatten().zip(piece.domains) {
                                    tracker.charge(domain.interval_count().checked_add(1)?)?;
                                    match operand {
                                        CallableHistoryOperand::Axis(_, axis) => {
                                            let entry = original.get(axis)?;
                                            let mut cost = entry.interval_count();
                                            original.charge_affine_replacement(axis, domain.interval_count(), &mut cost)?;
                                            tracker.charge(cost.checked_add(original.equal_axes().len())?)?;
                                            if domain.is_empty() || !domain.is_subset_of(entry) { return None; }
                                            next.replace_domain(axis, domain);
                                        }
                                        CallableHistoryOperand::Exact((_, value)) if domain.singleton_value() == Some(value) => {}
                                        _ => return None,
                                    }
                                }
                                for step in piece.steps { tracker.append(&mut next, step); }
                                tracker.append(&mut next, returned);
                                survivors.0.push(next);
                            }
                        }
                        path = survivors;
                        continue;
                    }
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
                        let comparison = if let Some(compound) = path.callable_compound_branch(calls, *condition, roots)? {
                            Some(compound)
                        } else if evaluated_guard.0.is_some() {
                            evaluated_guard.0
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
                    #[cfg(feature = "internal-checker")]
                    if let Some((source, trace)) = &roots.startup_trace {
                        let ordinary_bool = trace.borrow().admits_typed_ordinary_startup()
                            && matches!(structure.checker_ordinary_booleans.get(condition),
                                Some(super::reference::CheckerOrdinaryBoolean::Binding(_)));
                        if evaluated_guard.1.is_some()
                            || !(ordinary_bool || matches!(structure.checker_branches.get(condition),
                                Some(CheckerPredicate::Boolean(_) | CheckerPredicate::Integer(_))))
                            || yes.is_unreachable() == no.is_unreachable()
                        { return None; }
                        let at = calls.occurrences.get(condition.0)?.span;
                        trace.try_borrow_mut().ok()?.branch(wosy_syntax::SourceSpan::new((*source).clone(), at), !yes.is_unreachable())?;
                    }
                    #[cfg(feature = "internal-checker")]
                    let (yes, no, keep_arm_rows) = if let Some(histories) = roots.callable_history {
                        let mut yes = yes;
                        let mut no = no;
                        let at = calls.occurrences.get(condition.0)?.span;
                        let mut histories = histories.borrow_mut();
                        histories.append_rows(&mut yes, at, true)?;
                        histories.append_rows(&mut no, at, false)?;
                        (yes, no, true)
                    } else { (yes, no, keep_arm_rows) };
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
                #[cfg(feature = "internal-checker")]
                if Authored::OBSERVE_RESULT_PAIR {
                    authored_return.observe_result_pair(calls, block, slot, roots, &path)?;
                    roots.truncate(depth);
                    return Some(IntegerControl::Returned);
                }
                let ty = structure.checked_types.expressions.get(&slot.expression)?;
                if let Some(expected) = tail {
                    #[cfg(feature = "internal-checker")]
                    let array = Authored::OBSERVE_FIXED_ARRAY && matches!(ty, ScalarType::Array { .. })
                        && matches!(expected, ScalarType::Array { .. }) && super::scalar_type_equal(ty, expected);
                    #[cfg(not(feature = "internal-checker"))]
                    let array = false;
                    if slot.output != 0 || (ty != expected && !array) {
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
                        let comparison = if evaluated_guard.0.is_some() {
                            evaluated_guard.0
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
                    #[cfg(feature = "internal-checker")]
                    if let Some((source, trace)) = &roots.startup_trace {
                        let ordinary_bool = trace.borrow().admits_typed_ordinary_startup()
                            && matches!(structure.checker_ordinary_booleans.get(condition),
                                Some(super::reference::CheckerOrdinaryBoolean::Binding(_)));
                        if evaluated_guard.1.is_some()
                            || !(ordinary_bool || matches!(structure.checker_branches.get(condition),
                                Some(CheckerPredicate::Boolean(_) | CheckerPredicate::Integer(_))))
                            || yes.is_unreachable() == no.is_unreachable()
                        { return None; }
                        let at = calls.occurrences.get(condition.0)?.span;
                        trace.try_borrow_mut().ok()?.branch(wosy_syntax::SourceSpan::new((*source).clone(), at), !yes.is_unreachable())?;
                    }
                    #[cfg(feature = "internal-checker")]
                    let (yes, no) = if let Some(histories) = roots.callable_history {
                        let mut yes = yes;
                        let mut no = no;
                        let at = calls.occurrences.get(condition.0)?.span;
                        let mut histories = histories.borrow_mut();
                        histories.append_rows(&mut yes, at, true)?;
                        histories.append_rows(&mut no, at, false)?;
                        (yes, no)
                    } else { (yes, no) };
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
                if Authored::OBSERVE_FIXED_ARRAY {
                    authored_return.observe_fixed_array(calls, slot, ty, roots, &path)?;
                    if let Some((source, trace)) = &roots.startup_trace {
                        let [_] = path.0.as_slice() else { return None; };
                        trace.try_borrow_mut().ok()?.append(super::checker_project::failure_search::PathStep::Output(
                            wosy_syntax::SourceSpan::new((*source).clone(), slot.span)))?;
                    }
                    roots.truncate(depth);
                    return Some(IntegerControl::Returned);
                }
                #[cfg(feature = "internal-checker")]
                if roots.allow_real_integer_tail_call && COLLECT && SUSPEND_CALL && suspend_integer
                    && structure.checker_direct_targets.contains_key(&slot.expression)
                {
                    // Only the outer symbolic return can discard the child's
                    // exact partition without needing to compose it again.
                    let symbolic_root = discarded_return.as_deref().copied() == Some(false)
                        && block == structure.root && branches.is_empty();
                    if !tail.is_some_and(is_integer_type) || !terminal.is_empty()
                        || (!symbolic_root && !returns.as_ref().is_some_and(|completed| completed.is_empty()))
                    {
                        return None;
                    }
                    let [row] = path.0.as_slice() else { return None; };
                    if row.affine.is_some() || row.entry.is_some() { return None; }
                    charge_call_work(roots.ordinary_local_work.get_or_insert(0), 1)?;
                    *cursor = IntegerBlockCursor { block, next, depth, path, branches, terminal, guard };
                    return Some(IntegerControl::PendingCall(ExactCallEffect::TailInteger {
                        call: slot.expression, ty,
                    }));
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
                            if roots.named_pair_rectangles {
                                row.require_named_pair_entry(roots.named_pair_bool_axis, work)?;
                            }
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
                                    span: slot.span,
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
                let value = if let Some(value) = plain_typed_return(calls, &slot, ty, roots) {
                    value
                } else if Authored::ENABLED && tail.is_some() {
                    IntegerReturn::Const(authored_return.at(calls, block, slot, ty, roots, &path)?)
                } else {
                    return None;
                };
                if tail.is_some() {
                    #[cfg(feature = "internal-checker")]
                    if let Some((source, trace)) = &roots.startup_trace {
                        let [_] = path.0.as_slice() else { return None; };
                        trace.try_borrow_mut().ok()?.append(super::checker_project::failure_search::PathStep::Output(
                            wosy_syntax::SourceSpan::new((*source).clone(), slot.span)))?;
                    }
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
                #[cfg(feature = "internal-checker")]
                if roots.allow_extended_scalar_locals && roots.allow_ordinary_contextual_places
                    && matches!(calls.boundary.results.as_slice(), [result] if result.ty == ScalarType::Unit)
                {
                    if let Some(update) = structure.checker_contextual_counter_updates.get(&(group.block, group.item)) {
                        let (binding, ty, rhs, destination) = checked_scalar_assignment(calls, group)?;
                        let (axis, active_ty) = roots.lookup(binding)?;
                        if active_ty != ty || !is_integer_type(ty) || update.rhs != rhs
                            || !structure.checker_contextual_occurrences.contains(&destination)
                        {
                            return None;
                        }
                        use super::reference::CheckerContextualCounterOperand::{Lexical, Literal};
                        let exact;
                        let amount = match &update.amount {
                            Literal(value) => value,
                            Lexical(binding) => {
                                if roots.symbolic_context_entry_amount {
                                    let (argument, actual_ty) = roots.lookup(binding)?;
                                    if actual_ty != ty || argument == axis { return None; }
                                    let operation = AuthoredIntegerOperation::checked(&update.operator)?;
                                    let (min, max) = super::typecheck::integer_bounds(ty)?;
                                    let work = roots.symbolic_context_work.as_mut()?;
                                    for row in &mut path.0 {
                                        if row.affine.is_some() || row.entry.is_some() { return None; }
                                        charge_call_work(work, row.len().checked_add(row.equal_axes().len())?.checked_add(5)?)?;
                                        let amount = row.get(argument)?.singleton_value()?;
                                        if amount.bits() == 0 {
                                            // The prepared ordinary Add/Subtract by
                                            // zero leaves the exact place unchanged.
                                            continue;
                                        }
                                        let entry = row.get(axis)?.singleton_value()?;
                                        let exit = operation.apply(Cow::Borrowed(entry), Cow::Borrowed(amount), &min, &max)?;
                                        if exit < min || exit > max { return None; }
                                        row.overwrite(axis, Rc::new(IntegerSet::singleton(&exit)), false);
                                    }
                                    continue;
                                }
                                // One original checked actual row only. Do not
                                // project a symbolic operand independently of
                                // its contextual entry or reuse across calls.
                                if !SUSPEND_CALL || !roots.allow_exact_contextual_counter_argument { return None; }
                                let (binding_axis, binding_ty) = roots.lookup(binding)?;
                                let [row] = path.0.as_slice() else { return None; };
                                if binding_ty != ty || binding_axis == axis { return None; }
                                charge_call_work(roots.ordinary_local_work.get_or_insert(0), 1)?;
                                exact = row.get(binding_axis)?.singleton_value()?.clone();
                                &exact
                            }
                        };
                        path = path.finite_ordinary_calculation(
                            AuthoredIntegerOperation::checked(&update.operator)?, (None, Some(axis)),
                            (Some(amount), None), ty, FiniteIntegerInstall::OverwriteSource,
                            roots.ordinary_local_work.get_or_insert(0),
                        )?;
                        continue;
                    }
                }
                #[cfg(feature = "internal-checker")]
                if roots.allow_symbolic_contextual_bool {
                    if let Some((binding, ScalarType::Bool, rhs, destination)) = checked_scalar_assignment(calls, group) {
                        if structure.checker_contextual_occurrences.contains(&destination) {
                            let (axis, ty) = roots.lookup(binding)?;
                            if *ty != ScalarType::Bool { return None; }
                            let operand = contextual_boolean_source(calls, roots, rhs)?;
                            let width = roots.axes.len();
                            let work = roots.ordinary_local_work.get_or_insert(0);
                            charge_call_work(work, width.checked_add(1)?)?;
                            let mut changes = std::iter::repeat_with(|| None).take(width).collect::<Vec<_>>();
                            changes[axis] = Some(operand);
                            path = path.parallel_overwrite(&changes, work)?;
                            continue;
                        }
                    }
                }
                #[cfg(feature = "internal-checker")]
                if roots.allow_exact_contextual_bool {
                    if let Some((binding, ScalarType::Bool, rhs, destination)) = checked_scalar_assignment(calls, group) {
                        if structure.checker_contextual_occurrences.contains(&destination) {
                            let (axis, ty) = roots.lookup(binding)?;
                            if *ty != ScalarType::Bool { return None; }
                            let [row] = path.0.as_mut_slice() else { return None; };
                            if row.affine.is_some() || row.entry.is_some() { return None; }
                            bool_result_lane(row.get(axis)?.singleton_value()?)?;
                            charge_call_work(roots.ordinary_local_work.get_or_insert(0),
                                4usize.checked_add(row.equal_axes().len().checked_mul(3)?)?)?;
                            let value = exact_contextual_boolean_value(calls, roots, row, rhs)?;
                            row.overwrite(axis, value, false);
                            continue;
                        }
                    }
                }
                #[cfg(feature = "internal-checker")]
                if roots.projected_scalar_local_write {
                    if let Some(write) = projected_scalar_local_write(calls, group) {
                        path = apply_projected_scalar_local_write(calls, roots, path, write)?;
                        continue;
                    }
                }
                #[cfg(feature = "internal-checker")]
                if roots.exact_returning_local_writes {
                    path = exact_returning_local_write(calls, roots, group, path)?;
                    continue;
                }
                #[cfg(feature = "internal-checker")]
                if calls.assignments.get(&(group.block, group.item))?.destinations().len() > 1 {
                    if !COLLECT || USE_IMPORTED_BOUND { return None; }
                    path = ordinary_parallel_assignment(calls, roots, group, path)?;
                    continue;
                }
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
                                || (structure.checker_contextual_occurrences.contains(&destination)
                                    && !roots.allow_ordinary_contextual_places)
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
                if axis < roots.parameters && !preserve_entry {
                    #[cfg(feature = "internal-checker")]
                    let has_blocking_alias = if structure.checker_selected_snapshots.is_empty() {
                        path.axis_has_alias(axis)
                    } else {
                        // Hidden aliases preserve a construction-time capture,
                        // not a second authored view of the input formal.
                        path.axis_has_visible_alias(axis, roots)?
                    };
                    #[cfg(not(feature = "internal-checker"))]
                    let has_blocking_alias = path.axis_has_alias(axis);
                    if has_blocking_alias { return None; }
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
                    #[cfg(feature = "internal-checker")]
                    if origin.arguments.as_slice().is_empty() { return None; }
                    let [receiver] = binding.receivers() else {
                        return None;
                    };
                    use ImportedScalarArgument as Argument;
                    #[cfg(feature = "internal-checker")]
                    let mut arguments = [None; super::checker_project::MAX_RANKED_INTEGER_AXES];
                    #[cfg(not(feature = "internal-checker"))]
                    let mut arguments = [None; 2];
                    for (slot, (ordinal, argument)) in origin.arguments.as_slice().iter().enumerate() {
                        *arguments.get_mut(slot)? = Some(match argument {
                            CheckerIntegerCallArgument::Literal(value) => Argument::Literal(value),
                            CheckerIntegerCallArgument::ContextualValue => return None,
                            #[cfg(feature = "internal-checker")]
                            CheckerIntegerCallArgument::RankedSelfDescent | CheckerIntegerCallArgument::ExactBoolean => return None,
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
                        ImportedIntegerValues::Shared { result, guards, domain, meter_rectangle, full_ranked_meter } => {
                            if meter_rectangle {
                                let work = roots.ordinary_local_work.get_or_insert(0);
                                let width = origin.arguments.as_slice().len();
                                charge_call_work(work, 1usize.checked_add(guards.len())?
                                    .checked_add(if full_ranked_meter { width } else { 0 })?)?;
                                for row in &path.0 {
                                    let mut cost = row.len().checked_add(1)?;
                                    if full_ranked_meter {
                                        // The per-row accounting loop visits literal
                                        // guards as well as guards on caller axes.
                                        cost = cost.checked_add(guards.len())?;
                                    }
                                    if matches!(result, ImportedVerifiedReturn::EntryAlias(_) | ImportedVerifiedReturn::EntryAffine { .. }) {
                                        cost = cost.checked_add(row.equal_axes().len().checked_mul(2)?)?.checked_add(2)?;
                                    }
                                    for guard in guards {
                                        if let Some(Argument::Axis(axis)) = arguments.get(guard.slot).copied().flatten() {
                                            cost = cost.checked_add(row.get(axis)?.interval_count())?;
                                        }
                                    }
                                    if full_ranked_meter {
                                        if let Some((slot, domain)) = domain {
                                            if let Some(Argument::Axis(axis)) = arguments.get(slot).copied().flatten() {
                                                cost = cost.checked_add(row.get(axis)?.interval_count())?
                                                    .checked_add(domain.interval_count())?;
                                            }
                                        }
                                    }
                                    charge_call_work(work, cost)?;
                                }
                            }
                            if !guards.iter().all(applies) || domain.is_some_and(|domain| !in_domain(domain)) {
                                return None;
                            }
                            let slot = match &result {
                                ImportedVerifiedReturn::Constant { .. } | ImportedVerifiedReturn::PairPartitions(_) => None,
                                ImportedVerifiedReturn::EntryAlias(slot) | ImportedVerifiedReturn::EntryAffine { slot, .. } => Some(*slot),
                                ImportedVerifiedReturn::UnaryPartitions(_) => Some(0),
                            };
                            let alias = if let Some(slot) = slot {
                                let (argument, _) = origin.arguments.as_slice().get(slot)?;
                                if !meter_rectangle || structure.checked_types.expressions.get(argument) != Some(&receiver.ty) {
                                    return None;
                                }
                                Some(arguments.get(slot).copied().flatten()?)
                            } else { None };
                            roots.add(&origin.binding, &receiver.ty);
                            match (result, alias) {
                                (ImportedVerifiedReturn::Constant { values, bound }, None) => {
                                    let values = values.get_or_init(|| Rc::new(IntegerSet::singleton(bound)));
                                    path.append_shared_local(values);
                                }
                                (ImportedVerifiedReturn::EntryAlias(_), Some(Argument::Literal(value))) =>
                                    path.append_local(IntegerSet::singleton(value)),
                                (ImportedVerifiedReturn::EntryAlias(_), Some(Argument::Axis(axis))) => {
                                    for row in &mut path.0 {
                                        let domain = Rc::clone(row.get(axis)?);
                                        row.append_alias(axis, domain);
                                    }
                                }
                                (ImportedVerifiedReturn::EntryAffine { transform, .. }, Some(argument)) => {
                                    let (min, max) = super::typecheck::integer_bounds(&receiver.ty)?;
                                    let work = roots.affine_work.get_or_insert(0);
                                    match argument {
                                        Argument::Literal(value) => {
                                            let result = transform.apply_point(value);
                                            if result < min || result > max { return None; }
                                            path.append_local(IntegerSet::singleton(&result));
                                        }
                                        Argument::Axis(axis) => {
                                            for row in &mut path.0 {
                                                let source = row.get(axis)?;
                                                transform.fits(source, &min, &max)?;
                                                let plan = row.prepare_affine(axis, source, transform, work)?;
                                                row.append_affine(plan);
                                            }
                                        }
                                    }
                                }
                                (ImportedVerifiedReturn::UnaryPartitions(pieces), Some(Argument::Literal(value))) => {
                                    let work = roots.ordinary_local_work.get_or_insert(0);
                                    let returned = singleton_return_from_partitions(pieces, value, &receiver.ty, work)?;
                                    path.append_local(IntegerSet::singleton(&returned));
                                }
                                (ImportedVerifiedReturn::UnaryPartitions(pieces), Some(Argument::Axis(axis))) => {
                                    let mut joined = ScalarOutputRows::Bound(IntegerInputs::default());
                                    transfer_unary_return_partitions(&path.0, axis, roots.axes.get(axis)?.1,
                                        &receiver.ty, roots.axes.len(), pieces, &mut joined, &mut roots.affine_work,
                                        Some(roots.ordinary_local_work.get_or_insert(0)))?;
                                    let ScalarOutputRows::Bound(rows) = joined else { return None; };
                                    path = rows;
                                }
                                (ImportedVerifiedReturn::PairPartitions(pair), None) => {
                                    let (Some(first), Some(second)) = (arguments[0], arguments[1]) else { return None; };
                                    let [(left, _), (right, _)] = origin.arguments.as_slice() else { return None; };
                                    let types = [structure.checked_types.expressions.get(left)?, structure.checked_types.expressions.get(right)?];
                                    path = transfer_imported_pair_returns(&path.0, [first, second], types, &receiver.ty, pair,
                                        &mut roots.affine_work, roots.ordinary_local_work.get_or_insert(0))?;
                                }
                                _ => return None,
                            }
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
                    if origin.arguments.as_slice().len() > 2 {
                        // Wide canonical calls are only consumed as enrolled
                        // ranked imports above, never by the generic evaluator.
                        return None;
                    }
                    let [receiver] = binding.receivers() else { return None; };
                    let contextual = matches!(origin.arguments.as_slice(), [(_, CheckerIntegerCallArgument::ContextualValue)]);
                    #[cfg(feature = "internal-checker")]
                    let ordinary_contextual = roots.allow_ordinary_contextual_places;
                    #[cfg(not(feature = "internal-checker"))]
                    let ordinary_contextual = false;
                    if SUSPEND_CALL && suspend_integer && (!contextual || ordinary_contextual) {
                        *cursor = IntegerBlockCursor { block, next, depth, path, branches, terminal, #[cfg(feature = "internal-checker")] guard };
                        return Some(IntegerControl::PendingCall(ExactCallEffect::Scalar {
                            call: origin.call, binding: Some(&origin.binding), ty: &receiver.ty,
                        }));
                    }
                    #[cfg(feature = "internal-checker")]
                    if origin.arguments.as_slice().is_empty()
                        || origin.arguments.as_slice().iter().any(|(_, argument)| matches!(argument,
                            CheckerIntegerCallArgument::RankedSelfDescent | CheckerIntegerCallArgument::ExactBoolean))
                    {
                        // Nullary, ranked and Bool typed origins are consumed
                        // only by the explicitly gated exact stack.
                        return None;
                    }
                    let handle = handle_call.as_mut()?;
                    let signature = calls.checked_call_signatures.get(&origin.call)?;
                    #[cfg(feature = "internal-checker")]
                    if let Some(histories) = roots.callable_history {
                        if contextual || !(1..=2).contains(&signature.parameters.len()) { return None; }
                        let span = calls.occurrences.get(origin.call.0)?.span;
                        let mut survivors = IntegerInputs::default();
                        for mut row in std::mem::take(&mut path.0) {
                            let mut tracker = histories.borrow_mut();
                            tracker.charge(row.len().checked_add(3)?)?;
                            let call = tracker.boundary(span, false);
                            tracker.append(&mut row, call);
                            tracker.outgoing_call = Some(tracker.steps(row.callable_history)?);
                            drop(tracker);
                            let exact = IntegerInputs(vec![row]);
                            let argument = if signature.parameters.len() == 2 {
                                nested_pair_argument(calls, origin.call, roots, &exact, true, false, true, false, None)?
                            } else {
                                actual_nested_argument(calls, origin.call, roots, &exact, true, false, None)
                            };
                            let operands = argument.callable_history_operands()?;
                            let result = handle(calls, origin.call, signature, argument, &receiver.ty)?;
                            let mut tracker = histories.borrow_mut();
                            let steps = tracker.scalar_return_steps.take()?;
                            if let [Some(first), Some(second)] = operands {
                                let ActualCallResult::PartitionedIntegerPair(pair) = result else { return None; };
                                let axes = [first, second].map(|operand| match operand {
                                    CallableHistoryOperand::Axis(_, axis) => Some(axis), CallableHistoryOperand::Exact(_) => None,
                                });
                                if pair.caller_axes != [0, 1] || pair.pieces.len() != steps.len()
                                    || (axes == [None, None] && steps.len() != 1)
                                { return None; }
                                let types = [&signature.parameters[0], &signature.parameters[1]];
                                let original = exact.0.first()?;
                                for (piece, steps) in pair.pieces.into_iter().zip(steps) {
                                    if piece.anchor != 1 { return None; }
                                    tracker.charge(steps.len().checked_add(1)?)?;
                                    let mut work = tracker.work.get();
                                    let mut output = ScalarOutputRows::Bound(IntegerInputs::default());
                                    let affine = attach_pair_return(original, axes, types, &receiver.ty,
                                        piece.inputs, &piece.output, &mut output, &mut work)?;
                                    tracker.work.set(work);
                                    if affine { roots.affine_work.get_or_insert(0); }
                                    let ScalarOutputRows::Bound(rows) = output else { return None; };
                                    for mut row in rows.0 {
                                        for step in &steps { tracker.append(&mut row, *step); }
                                        let returned = tracker.boundary(span, true);
                                        tracker.append(&mut row, returned);
                                        survivors.0.push(row);
                                    }
                                }
                                continue;
                            }
                            let (axis, value) = match operands {
                                [Some(CallableHistoryOperand::Axis(_, axis)), None] => (Some(axis), None),
                                [Some(CallableHistoryOperand::Exact((_, value))), None] => (None, Some(value)),
                                _ => return None,
                            };
                            let ActualCallResult::PartitionedInteger(pieces) = result else { return None; };
                            if pieces.len() != steps.len() || (axis.is_none() && pieces.len() != 1) { return None; }
                            for (piece, steps) in pieces.into_iter().zip(steps) {
                                tracker.charge(steps.len().checked_add(1)?)?;
                                let mut work = tracker.work.get();
                                let returned = if let Some(axis) = axis {
                                    let mut joined = ScalarOutputRows::Bound(IntegerInputs::default());
                                    transfer_unary_return_partitions(&exact.0, axis, roots.axes.get(axis)?.1,
                                        &receiver.ty, roots.axes.len(), std::slice::from_ref(&piece), &mut joined,
                                        &mut roots.affine_work, Some(&mut work))?;
                                    let ScalarOutputRows::Bound(rows) = joined else { return None; };
                                    rows
                                } else {
                                    let literal = singleton_return_from_partitions(std::slice::from_ref(&piece),
                                        value?, &receiver.ty, &mut work)?;
                                    charge_call_work(&mut work, exact.0.first()?.len())?;
                                    let mut rows = IntegerInputs(exact.0.clone());
                                    rows.append_local(IntegerSet::singleton(&literal));
                                    rows
                                };
                                tracker.work.set(work);
                                for mut row in returned.0 {
                                    for step in &steps { tracker.append(&mut row, *step); }
                                    let returned = tracker.boundary(span, true);
                                    tracker.append(&mut row, returned);
                                    survivors.0.push(row);
                                }
                            }
                        }
                        roots.add(&origin.binding, &receiver.ty);
                        path = survivors;
                        continue;
                    }
                    let argument = actual_nested_argument(calls, origin.call, roots, &path, false, false, None);
                    #[cfg(feature = "internal-checker")]
                    let argument = if contextual {
                        let NestedIntegerArgument::Single(Some(exact)) = argument else { return None; };
                        NestedIntegerArgument::ContextualValue(exact)
                    } else {
                        argument
                    };
                    #[cfg(not(feature = "internal-checker"))]
                    if contextual {
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
                        #[cfg(feature = "internal-checker")]
                        if receiver.ty == ScalarType::Bool && roots.allow_symbolic_contextual_bool {
                            let BindingInitialization::Outputs(outputs) = &binding.initialization else { return None; };
                            if !outputs.secondary.is_empty() || structure.checker_integer_locals.get(&(block, item.position))? != identity {
                                return None;
                            }
                            match contextual_boolean_source(calls, roots, outputs.primary)? {
                                ParallelScalarSource::Copy(axis) => path.append_ordinary_copy(axis, roots)?,
                                ParallelScalarSource::Literal(value) => {
                                    let work = roots.ordinary_local_work.get_or_insert(0);
                                    for row in &path.0 {
                                        let growth = if row.len() == row.cells.capacity() { row.len() } else { 0 };
                                        charge_call_work(work, growth.checked_add(1)?)?;
                                    }
                                    path.append_shared_local(&value);
                                }
                            }
                            roots.add(identity, &receiver.ty);
                            continue;
                        }
                        #[cfg(feature = "internal-checker")]
                        if receiver.ty == ScalarType::Bool && roots.allow_exact_contextual_bool {
                            let BindingInitialization::Outputs(outputs) = &binding.initialization else { return None; };
                            if !outputs.secondary.is_empty() { return None; }
                            let registered = structure.checker_integer_locals.get(&(block, item.position))?;
                            if registered != identity { return None; }
                            let [row] = path.0.as_slice() else { return None; };
                            if row.affine.is_some() || row.entry.is_some() { return None; }
                            let growth = row.len().checked_mul(3)?.checked_add(4)?;
                            charge_call_work(roots.ordinary_local_work.get_or_insert(0), growth)?;
                            let value = exact_contextual_boolean_value(calls, roots, row, outputs.primary)?;
                            roots.add(identity, &receiver.ty);
                            path.append_shared_local(&value);
                            continue;
                        }
                        if !super::typecheck::is_integer_type(&receiver.ty) {
                            return None;
                        }
                        let BindingInitialization::Outputs(outputs) = &binding.initialization else { return None; };
                        #[cfg(feature = "internal-checker")]
                        if roots.allow_ordinary_contextual_places
                            && structure.checker_contextual_declarations.contains(&(block, item.position))
                            && !structure.checker_integer_literals.contains_key(&outputs.primary)
                        {
                            let PlainNameIdentity::Lexical(source) = calls.prepared_plain_name(outputs.primary)? else { return None; };
                            let (axis, ty) = roots.lookup(source)?;
                            if ty != &receiver.ty || !outputs.secondary.is_empty()
                                || structure.checked_types.expressions.get(&outputs.primary)? != ty
                            {
                                return None;
                            }
                            let identity = structure.checker_integer_locals.get(&(block, item.position))
                                .filter(|prepared| *prepared == identity)?;
                            // The prepared source denotes the still-active outer
                            // place (or lexical value) before this declaration.
                            path.append_ordinary_copy(axis, roots)?;
                            roots.add(identity, &receiver.ty);
                            continue;
                        }
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
                if structure.checker_selected_snapshots.contains(&(block, item.position))
                    || ((Authored::OBSERVE_FIXED_ARRAY || Authored::OBSERVE_RUNTIME_FIXED_INDEX)
                        && (structure.checker_snapshot_locals.contains_key(&(block, item.position))
                        || structure.checker_snapshot_copies.contains_key(&(block, item.position))))
                {
                    // Legacy early-failure checks do not materialize authored state.
                    // Array proof observations borrow the live captures without publishing axes.
                    if !COLLECT && !Authored::OBSERVE_FIXED_ARRAY { return None; }
                    // Exact typed aggregate copies share the already-live
                    // construction captures. They add no witness columns.
                    if structure.checker_snapshot_copies.contains_key(&(block, item.position)) { continue; }
                    let shape = structure.checker_snapshot_locals.get(&(block, item.position))?;
                    if shape.has_affine_leaf && !roots.allow_extended_scalar_locals && !roots.allow_snapshot_history { return None; }
                    use super::checker_directive::CheckerSnapshotOperand;
                    for field in shape.leaves.iter() {
                        match &field.operand {
                            CheckerSnapshotOperand::Binding(binding) => {
                                let (axis, ty) = roots.lookup(binding)?;
                                if ty != &field.ty { return None; }
                                path.append_ordinary_copy(axis, roots)?;
                            }
                            CheckerSnapshotOperand::Affine { source, operator, literal, source_on_left } => {
                                let (operation, left, right) = checked_snapshot_operation(
                                    roots, &field.ty, source, operator, literal, *source_on_left,
                                )?;
                                path.append_ordinary_calculation(operation, left, right,
                                    &field.ty, &mut roots.ordinary_local_work, &mut roots.affine_work)?;
                            }
                            CheckerSnapshotOperand::Literal(value) => {
                                let work = roots.ordinary_local_work.get_or_insert(0);
                                charge_call_work(work, path.0.len().checked_add(1)?)?;
                                path.append_local(IntegerSet::singleton(value));
                            }
                        }
                        roots.add(&field.capture, &field.ty);
                    }
                    continue;
                }
                #[cfg(feature = "internal-checker")]
                let exact_imported_match = block == structure.root
                    && authored_return.imported_enum_match_position() == Some(item.position);
                #[cfg(feature = "internal-checker")]
                let history_match = roots.callable_history.is_some()
                    && structure.checker_ordinary_scalar_locals.get(&(block, item.position)).is_some_and(|origin|
                        matches!(&origin.kind, super::reference::CheckerOrdinaryScalarLocalKind::EnumMatch { qualified_prefix: false, .. }));
                #[cfg(feature = "internal-checker")]
                let history_snapshot_read = roots.allow_snapshot_history
                    && structure.checker_ordinary_scalar_locals.get(&(block, item.position)).is_some_and(|origin|
                        matches!(&origin.kind, super::reference::CheckerOrdinaryScalarLocalKind::SnapshotRead {
                            index: super::reference::CheckerSnapshotReadIndex::Leaf(_), .. }));
                #[cfg(feature = "internal-checker")]
                if roots.allow_ordinary_integer_locals || exact_imported_match || history_match || history_snapshot_read {
                    if let Some(origin) = structure.checker_ordinary_scalar_locals.get(&(block, item.position)) {
                        let [receiver] = binding.receivers() else { return None; };
                        let BindingInitialization::Outputs(outputs) = &binding.initialization else { return None; };
                        if origin.source != outputs.primary || !outputs.secondary.is_empty()
                            || receiver.contextual_storage.is_some()
                            || structure.checked_types.expressions.get(&origin.source)? != &receiver.ty
                        {
                            return None;
                        }
                        if exact_imported_match && !matches!(&origin.kind,
                            super::reference::CheckerOrdinaryScalarLocalKind::EnumMatch { qualified_prefix: true, .. })
                        {
                            return None;
                        }
                        match &origin.kind {
                            super::reference::CheckerOrdinaryScalarLocalKind::Copy => {
                                let PlainNameIdentity::Lexical(source) = calls.prepared_plain_name(origin.source)? else { return None; };
                                let (axis, ty) = roots.lookup(source)?;
                                if ty != &receiver.ty { return None; }
                                // Read the old binding before installing the new one.
                                path.append_ordinary_copy(axis, roots)?;
                            }
                            super::reference::CheckerOrdinaryScalarLocalKind::SnapshotRead { shape, index } => {
                                use super::reference::{CheckerSnapshotArrayOffset, CheckerSnapshotReadIndex};
                                if !roots.allow_extended_scalar_locals && !history_snapshot_read { return None; }
                                match index {
                                    CheckerSnapshotReadIndex::Leaf(leaf) => {
                                        let field = shape.leaves.get(*leaf)?;
                                        let (axis, ty) = roots.lookup(&field.capture)?;
                                        if ty != &receiver.ty || field.ty != *ty { return None; }
                                        path.append_ordinary_copy(axis, roots)?;
                                    }
                                    CheckerSnapshotReadIndex::ArrayPath(prepared) => {
                                        charge_call_work(roots.ordinary_local_work.get_or_insert(0), prepared.len().checked_add(1)?)?;
                                        let mut steps = Vec::with_capacity(prepared.len());
                                        for step in prepared.iter() {
                                            let index = match step.offset {
                                                CheckerSnapshotArrayOffset::Literal(index) => SnapshotIndexValue::Literal(index),
                                                CheckerSnapshotArrayOffset::Lexical(index) => {
                                                    let PlainNameIdentity::Lexical(binding) = calls.prepared_plain_name(index)?
                                                        else { return None; };
                                                    let (axis, ty) = roots.lookup(binding)?;
                                                    if *ty != ScalarType::U64 { return None; }
                                                    SnapshotIndexValue::Axis(axis)
                                                }
                                            };
                                            steps.push((step.length, index));
                                        }
                                        let by_binding = &roots.by_binding;
                                        let axes = &roots.axes;
                                        let field_axis = |leaf: usize| {
                                            let field = shape.leaves.get(leaf)?;
                                            let axis = *by_binding.get(&field.capture)?;
                                            (axes.get(axis)?.1 == &receiver.ty && field.ty == receiver.ty).then_some(axis)
                                        };
                                        path = path.bounded_snapshot_path(&shape.root, &steps, field_axis,
                                            roots.ordinary_local_work.get_or_insert(0))?;
                                        if Authored::OBSERVE_RUNTIME_FIXED_INDEX && !path.is_unreachable() {
                                            if let [step] = prepared.as_ref() {
                                                if matches!(step.offset, CheckerSnapshotArrayOffset::Lexical(_)) {
                                                    charge_call_work(roots.ordinary_local_work.get_or_insert(0), 1)?;
                                                    authored_return.observe_runtime_fixed_index(origin, step.length)?;
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            super::reference::CheckerOrdinaryScalarLocalKind::EnumMatch { transfer, qualified_prefix } => {
                                if if *qualified_prefix { !exact_imported_match } else { !roots.allow_extended_scalar_locals && !history_match } {
                                    return None;
                                }
                                path = bind_enum_match_value(roots, path, transfer, &receiver.ty)?;
                            }
                            super::reference::CheckerOrdinaryScalarLocalKind::Conditional => {
                                if !roots.allow_extended_scalar_locals { return None; }
                                path = bind_conditional_scalar_local(calls, roots, path, origin.source, &receiver.ty)?;
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
                                if matches!(operation, AuthoredIntegerOperation::Multiply | AuthoredIntegerOperation::Divide
                                    | AuthoredIntegerOperation::Remainder)
                                {
                                    if !roots.allow_extended_scalar_locals { return None; }
                                    path = path.finite_ordinary_calculation(operation, left, right, &receiver.ty,
                                        FiniteIntegerInstall::Append, roots.ordinary_local_work.get_or_insert(0))?;
                                } else {
                                    path.append_ordinary_calculation(operation, left, right, &receiver.ty,
                                        &mut roots.ordinary_local_work, &mut roots.affine_work)?;
                                }
                            }
                        }
                        roots.add(&origin.binding, &receiver.ty);
                        continue;
                    }
                }
                #[cfg(feature = "internal-checker")]
                if roots.allow_ordinary_contextual_places {
                    if let Some(identity) = structure.checker_contextual_read_locals.get(&(block, item.position)) {
                        let [receiver] = binding.receivers() else { return None; };
                        let BindingInitialization::Outputs(outputs) = &binding.initialization else { return None; };
                        if receiver.contextual_storage.is_some() || !outputs.secondary.is_empty()
                            || !structure.checker_contextual_occurrences.contains(&outputs.primary)
                        {
                            return None;
                        }
                        let PlainNameIdentity::Lexical(source) = calls.prepared_plain_name(outputs.primary)? else { return None; };
                        let (axis, ty) = roots.lookup(source)?;
                        if ty != &receiver.ty || structure.checked_types.expressions.get(&outputs.primary)? != ty {
                            return None;
                        }
                        path.append_ordinary_copy(axis, roots)?;
                        roots.add(identity, &receiver.ty);
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
    #[cfg(feature = "internal-checker")]
    TailInteger { call: OccurrenceOrdinal, ty: &'a ScalarType },
}

impl ExactCallEffect<'_> {
    pub(super) fn occurrence(self) -> OccurrenceOrdinal {
        match self {
            Self::Unit(call) | Self::Scalar { call, .. } => call,
            #[cfg(feature = "internal-checker")]
            Self::TailInteger { call, .. } => call,
        }
    }

    pub(super) fn expected(&self) -> &ScalarType {
        match self {
            Self::Unit(_) => &ScalarType::Unit,
            Self::Scalar { ty, .. } => ty,
            #[cfg(feature = "internal-checker")]
            Self::TailInteger { ty, .. } => ty,
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
pub(super) fn bool_result_lane(value: &BigInt) -> Option<usize> {
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

// Attach a paired return to the invocation's own caller row. A fresh call
// already supplies row-relative cells; a verified import intersects its
// declaration-relative cells with the call arguments before attaching them.
fn attach_pair_return(
    row: &IntegerRow, axes: [Option<usize>; 2], types: [&ScalarType; 2], ty: &ScalarType,
    cells: [Rc<IntegerSet>; 2], output: &IntegerPairPartitionOutput, joined: &mut ScalarOutputRows,
    work: &mut usize,
) -> Option<bool> {
    let intervals = [cells[0].interval_count(), cells[1].interval_count()];
    let base = row.len().checked_add(1)?
        .checked_add(3usize.checked_mul(row.equal_axes().len())?)?
        .checked_add(intervals[0])?.checked_add(intervals[1])?;
    charge_call_work(work, base)?;
    match axes {
        [Some(first), Some(second)] => row.charge_affine_pair([first, second], intervals, work)?,
        [Some(axis), None] => row.charge_affine_replacement(axis, intervals[0], work)?,
        [None, Some(axis)] => row.charge_affine_replacement(axis, intervals[1], work)?,
        [None, None] => (),
    }
    charge_call_work(work, 0)?;
    let mut affine = None;
    let mut fixed = None;
    match output {
        IntegerPairPartitionOutput::Singleton(output) => fixed = Some(Rc::clone(output)),
        IntegerPairPartitionOutput::InputAlias(slot) | IntegerPairPartitionOutput::InputAffine(slot, _) => {
            if types.get(*slot)? != &ty { return None; }
            charge_call_work(work, 1)?;
            let transform = match output {
                IntegerPairPartitionOutput::InputAffine(_, transform) => Some(transform),
                _ => None,
            };
            let cell = cells.get(*slot)?;
            if let Some(axis) = axes[*slot] {
                if let Some(transform) = transform {
                    let (min, max) = super::typecheck::integer_bounds(ty)?;
                    transform.fits(cell, &min, &max)?;
                    affine = Some(row.prepare_affine(axis, cell, transform, work)?);
                }
            } else {
                // Only the verified import can supply a literal argument.
                #[cfg(not(feature = "internal-checker"))]
                return None;
                #[cfg(feature = "internal-checker")]
                {
                    let input = cell.singleton_value()?;
                    fixed = Some(if let Some(transform) = transform {
                        let value = transform.apply_point(input);
                        if !super::typecheck::integer_literal_fits_type(&value, ty) { return None; }
                        Rc::new(IntegerSet::singleton(&value))
                    } else {
                        Rc::clone(cell)
                    });
                }
            }
        }
    }
    let mut row = row.clone();
    let [left, right] = cells;
    match axes {
        [Some(first), Some(second)] => row.replace_pair([first, second], [left, right])?,
        [Some(axis), None] => row.replace_domain(axis, left),
        [None, Some(axis)] => row.replace_domain(axis, right),
        [None, None] => (),
    }
    if let Some(output) = fixed {
        joined.push_singleton(row, output)?;
    } else if let Some(plan) = affine {
        joined.push_affine(row, plan)?;
        return Some(true);
    } else if let IntegerPairPartitionOutput::InputAlias(slot) = output {
        joined.push_alias(row, axes[*slot]?)?;
    } else {
        return None;
    }
    Some(false)
}

#[cfg(feature = "internal-checker")]
fn transfer_imported_pair_returns(
    rows: &[IntegerRow], arguments: [ImportedScalarArgument<'_>; 2], types: [&ScalarType; 2], ty: &ScalarType,
    pair: &ProjectedIntegerPair, retained_affine: &mut Option<usize>, import_meter: &mut usize,
) -> Option<IntegerInputs> {
    if pair.caller_axes != [0, 1] || !types.into_iter().all(is_integer_type) { return None; }
    use ImportedScalarArgument::{Axis, Literal};
    let axes = arguments.map(|argument| match argument { Axis(axis) => Some(axis), Literal(_) => None });
    let mut joined = ScalarOutputRows::Bound(IntegerInputs::default());
    let mut uses_affine = retained_affine.is_some();
    let work = retained_affine.as_mut().unwrap_or(import_meter);
    charge_call_work(work, rows.len().checked_mul(pair.pieces.len().checked_add(1)?)?)?;
    for row in rows {
        if let [Some(first), Some(second)] = axes {
            // These receipts were proved as independent rectangles. Do not
            // flatten an equality or affine link between the actual arguments.
            let lookup = (usize::BITS - row.equal_axes().len().leading_zeros()) as usize;
            charge_call_work(work, lookup.checked_mul(2)?.checked_add(MAX_AFFINE_MEMBERS + 1)?)?;
            if row.linked_axes(first, second) { return None; }
        }
        let mut matched = false;
        'pieces: for piece in &pair.pieces {
            if piece.anchor != 0 { return None; }
            let mut cells = [Rc::clone(&piece.inputs[0]), Rc::clone(&piece.inputs[1])];
            for (slot, cell) in cells.iter_mut().enumerate() {
                let comparison = match arguments[slot] {
                    Literal(value) => {
                        charge_call_work(work, cell.interval_count())?;
                        if !cell.contains(value) { continue 'pieces; }
                        if cell.singleton_value().is_none() { *cell = Rc::new(IntegerSet::singleton(value)); }
                        continue;
                    }
                    Axis(axis) => row.get(axis)?,
                };
                charge_call_work(work, comparison.interval_count().checked_add(cell.interval_count())?)?;
                match comparison.intersection(cell) {
                    IntegerIntersection::Unchanged => *cell = Rc::clone(comparison),
                    IntegerIntersection::Empty => continue 'pieces,
                    IntegerIntersection::Changed(values) => *cell = Rc::new(values),
                }
            }
            uses_affine |= attach_pair_return(row, axes, types, ty, cells, &piece.output, &mut joined, work)?;
            matched = true;
        }
        // Full guarded coverage was established at issuance. Do not silently
        // drop a caller row if its receipt unexpectedly has no overlap.
        if !matched { return None; }
    }
    if uses_affine {
        *retained_affine = Some(*work);
    }
    let ScalarOutputRows::Bound(rows) = joined else { return None; };
    Some(rows)
}

// Both a freshly executed projected call and an authenticated reuse of its
// exact partitions attach returns to the same immutable caller-entry row.
fn transfer_unary_return_partitions(
    rows: &[IntegerRow], axis: usize, entry_type: &ScalarType, ty: &ScalarType, cells: usize,
    pieces: &[IntegerReturnPartition], joined: &mut ScalarOutputRows,
    retained: &mut Option<usize>, import_meter: Option<&mut usize>,
) -> Option<()> {
    let (piece_intervals, depends_on_input, affine) = pieces.iter().try_fold(
        (0usize, false, false), |(sum, depends, affine), piece| Some((
            sum.checked_add(piece.input.interval_count())?,
            depends || !matches!(&piece.output, IntegerPartitionOutput::Singleton(_)),
            affine || matches!(&piece.output, IntegerPartitionOutput::InputAffine(_)),
        )))?;
    if depends_on_input && entry_type != ty {
        return None;
    }
    let row_intervals = rows.iter().try_fold(0usize, |sum, row| sum.checked_add(row[axis].interval_count()))?;
    let comparisons = row_intervals.checked_mul(pieces.len())?
        .checked_add(piece_intervals.checked_mul(rows.len())?)?;
    let bounds = if affine { Some(super::typecheck::integer_bounds(ty)?) } else { None };
    if affine || retained.is_some() {
        charge_call_work(retained.get_or_insert(0), comparisons.checked_add(rows.len().checked_mul(pieces.len())?)?)?;
    } else {
        let row_copies = rows.len().checked_mul(pieces.len())?.checked_mul(cells)?;
        let alias_work = rows.iter().try_fold(0usize, |sum, row| sum.checked_add(row.equal_axes().len()))?
            .checked_mul(pieces.len())?;
        let cost = row_copies.checked_add(comparisons)?.checked_add(alias_work)?;
        if let Some(meter) = import_meter {
            charge_call_work(meter, cost)?;
        } else if cost > MAX_PARTITION_TRANSFER_WORK {
            return None;
        }
    }
    for row in rows {
        for piece in pieces {
            let narrowed = match row[axis].intersection(&piece.input) {
                IntegerIntersection::Unchanged => None,
                IntegerIntersection::Empty => continue,
                IntegerIntersection::Changed(values) => Some(values),
            };
            let mut plan = None;
            if let Some(work) = retained.as_mut() {
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
    Some(())
}

#[cfg(feature = "internal-checker")]
fn singleton_return_from_partitions(
    pieces: &[IntegerReturnPartition], input: &BigInt, ty: &ScalarType, work: &mut usize,
) -> Option<BigInt> {
    let mut returned = None;
    for piece in pieces {
        charge_call_work(work, piece.input.interval_count().checked_add(1)?)?;
        if !piece.input.contains(input) { continue; }
        let value = match &piece.output {
            IntegerPartitionOutput::Singleton(output) => output.singleton_value()?.clone(),
            IntegerPartitionOutput::InputAlias => input.clone(),
            IntegerPartitionOutput::InputAffine(transform) => transform.apply_point(input),
        };
        if !super::typecheck::integer_literal_fits_type(&value, ty) { return None; }
        if let Some(prior) = &returned {
            if prior != &value { return None; }
        } else {
            returned = Some(value);
        }
    }
    returned
}

pub(super) struct ExactCallCursor<'a> {
    roots: IntegerRoots<'a>,
    block: IntegerBlockCursor,
    tail: Option<&'a ScalarType>,
    #[cfg(feature = "internal-checker")]
    allow_empty_scalar_return: bool,
    #[cfg(feature = "internal-checker")]
    tail_call_result: Option<ActualCallResult>,
    #[cfg(feature = "internal-checker")]
    contextual_entry: Option<(usize, BigInt)>,
    #[cfg(feature = "internal-checker")]
    named_affine_partition: bool,
    #[cfg(feature = "internal-checker")]
    named_pure_projection_leaf: bool,
    discarded_return: Option<bool>,
    returned: Vec<IntegerReturningPath<'a>>,
    computed_returns: Vec<(usize, ComputedIntegerReturn)>,
    return_capture_work: usize,
    projected_slot: Option<usize>,
    projected_pair: Option<[usize; 2]>,
}

impl<'a> ExactCallCursor<'a> {
    #[cfg(feature = "internal-checker")]
    pub(super) fn allow_snapshot_history(&mut self) {
        self.roots.allow_snapshot_history = true;
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn returns_startup_bool(&self) -> bool { self.tail == Some(&ScalarType::Bool) }

    #[cfg(feature = "internal-checker")]
    pub(super) fn track_startup_path(
        &mut self, source: &'a wosy_syntax::SourceIdentity,
        trace: Rc<std::cell::RefCell<super::checker_project::failure_search::StartUpSearch>>,
        allow_exact_bool: bool,
    ) -> Option<()> {
        if self.roots.startup_trace.is_some()
            || self.tail.is_some_and(|ty| !(is_integer_type(ty) || (allow_exact_bool && *ty == ScalarType::Bool)))
            || self.projected_slot.is_some() || self.projected_pair.is_some()
        { return None; }
        self.roots.startup_trace = Some((source, trace));
        Some(())
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn allow_empty_scalar_return(&mut self) {
        debug_assert!(self.projected_slot.is_none() && self.projected_pair.is_none());
        self.allow_empty_scalar_return = self.tail.is_some_and(|ty| is_integer_type(ty) || *ty == ScalarType::Bool);
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn allow_empty_projected_integer(&mut self) -> Option<()> {
        if self.projected_slot != Some(0) || self.projected_pair.is_some() || !self.tail.is_some_and(is_integer_type) {
            return None;
        }
        self.allow_empty_scalar_return = true;
        self.roots.projected_narrow_integer = true;
        Some(())
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn allow_projected_scalar_local_write(&mut self) -> Option<()> {
        if self.projected_slot != Some(0) || self.projected_pair.is_some() || !self.tail.is_some_and(is_integer_type) {
            return None;
        }
        self.roots.projected_scalar_local_write = true;
        Some(())
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn restrict_returning_writes_to_direct_locals(&mut self) {
        debug_assert!(self.projected_slot.is_none() && self.projected_pair.is_none());
        debug_assert!(self.tail.is_some_and(is_integer_type));
        self.roots.exact_returning_local_writes = true;
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn allow_exact_contextual_bool(&mut self) {
        debug_assert!(self.tail.is_none() && self.projected_slot.is_none() && self.projected_pair.is_none());
        self.roots.allow_exact_contextual_bool = true;
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn allow_real_integer_tail_call(&mut self) {
        debug_assert!(self.tail.is_some_and(is_integer_type)
            && self.projected_slot.is_none() && self.projected_pair.is_none());
        self.roots.allow_real_integer_tail_call = true;
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn discards_symbolic_integer_return(&self) -> bool {
        self.tail.is_some_and(is_integer_type) && self.discarded_return == Some(false)
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn allow_symbolic_contextual_bool(&mut self) {
        debug_assert!(self.projected_slot.is_none() && self.projected_pair.is_none());
        self.roots.allow_symbolic_contextual_bool = true;
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn allow_extended_scalar_locals(&mut self) {
        debug_assert!(self.projected_slot.is_none() && self.projected_pair.is_none());
        self.roots.allow_extended_scalar_locals = true;
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn allow_named_exact_bool_parameters(&mut self) -> Option<()> {
        if self.projected_slot.is_some() || self.projected_pair.is_some() { return None; }
        for (axis, (_, ty)) in self.roots.axes.iter().enumerate() {
            if **ty == ScalarType::Bool {
                bool_result_lane(self.block.path.singleton_at(axis)?)?;
                self.roots.named_exact_bool = true;
            }
        }
        Some(())
    }

    pub(super) fn new(calls: &'a FunctionCallSites, exact: &[&BigInt], expected: &ScalarType) -> Option<Self> {
        Self::with_row(calls, expected, exact_integer_entry_row(exact), None)
    }

    pub(super) fn arbitrary_root(calls: &'a FunctionCallSites, expected: &ScalarType) -> Option<Self> {
        if !matches!(*expected, ScalarType::Unit | ScalarType::Bool) && !is_integer_type(expected) { return None; }
        let row = calls.boundary.parameters.iter()
            .map(|parameter| ordinary_parameter_set(&parameter.ty, true).map(Rc::new))
            .collect::<Option<IntegerRow>>()?;
        let mut cursor = Self::with_row(calls, expected, row, None)?;
        // Check typed output on all reached paths, but an uncalled root
        // has no caller to receive its value or an issued summary.
        if *expected != ScalarType::Unit { cursor.discarded_return = Some(false); }
        Some(cursor)
    }

    pub(super) fn projected(calls: &'a FunctionCallSites, values: Rc<IntegerSet>, expected: &ScalarType) -> Option<Self> {
        Self::with_row(calls, expected, IntegerRow::new(vec![values]), Some(0))
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn observes_named_affine_partitions(&self) -> bool {
        self.named_affine_partition
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn observe_named_affine_partitions(&mut self, pure_projection_leaf: bool) -> Option<()> {
        if self.projected_slot != Some(0) || self.projected_pair.is_some() || !self.tail.is_some_and(is_integer_type) {
            return None;
        }
        self.named_affine_partition = true;
        self.named_pure_projection_leaf = pure_projection_leaf;
        Some(())
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn named_pair_root(
        calls: &'a FunctionCallSites, expected: &ScalarType, inputs: [IntegerSet; 2], bool_axis: Option<usize>,
    ) -> Option<Self> {
        let cells = inputs.map(Rc::new);
        let rows = if let Some(axis) = bool_axis {
            // Encode categorical false/true with separate singleton rows;
            // a numeric interval never represents an admitted Bool tag.
            let mut rows = Vec::with_capacity(2);
            for lane in [false, true] {
                let mut inputs = cells.clone();
                *inputs.get_mut(axis)? = Rc::new(IntegerSet::singleton(&BigInt::from(u8::from(lane))));
                rows.push(IntegerRow::new(Vec::from(inputs)));
            }
            rows
        } else {
            vec![IntegerRow::new(Vec::from(cells))]
        };
        let mut cursor = Self::with_rows(calls, expected, IntegerInputs(rows), None, Some([0, 1]))?;
        cursor.require_named_pair_rectangles(false, bool_axis)?;
        Some(cursor)
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn require_named_pair_rectangles(&mut self, pure_projection_leaf: bool, bool_axis: Option<usize>) -> Option<()> {
        if self.projected_pair.is_none() || self.projected_slot.is_some() || !self.tail.is_some_and(is_integer_type) {
            return None;
        }
        if let Some(axis) = bool_axis {
            if axis >= 2 || *self.roots.axes.get(axis)?.1 != ScalarType::Bool
                || !is_integer_type(self.roots.axes.get(1 - axis)?.1) || pure_projection_leaf
            {
                return None;
            }
            self.roots.named_exact_bool = true;
        }
        for row in &self.block.path.0 {
            row.require_named_pair_entry(bool_axis, &mut self.return_capture_work)?;
        }
        self.roots.named_pair_rectangles = true;
        self.roots.named_pair_bool_axis = bool_axis;
        self.named_pure_projection_leaf = pure_projection_leaf;
        Some(())
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
        Some(Self { roots, block, tail,
            #[cfg(feature = "internal-checker")]
            allow_empty_scalar_return: false,
            #[cfg(feature = "internal-checker")]
            tail_call_result: None,
            #[cfg(feature = "internal-checker")]
            contextual_entry: None,
            #[cfg(feature = "internal-checker")]
            named_affine_partition: false,
            #[cfg(feature = "internal-checker")]
            named_pure_projection_leaf: false,
            discarded_return: None, returned: Vec::new(), computed_returns: Vec::new(), return_capture_work: 0, projected_slot, projected_pair })
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn contextual_singleton(&self, binding: &ReferenceBindingId, ty: &ScalarType) -> Option<&BigInt> {
        let (axis, active_ty) = self.roots.lookup(binding)?;
        let [row] = self.block.path.0.as_slice() else { return None; };
        if active_ty != ty || row.affine.is_some() { return None; }
        row.get(axis)?.singleton_value()
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn enter_contextual_unit(
        &mut self, binding: &'a ReferenceBindingId, ty: &'a ScalarType, entry: BigInt, guarded_resource: bool,
    ) -> Option<()> {
        let axis = self.roots.axes.len();
        if axis > 1 || axis != self.roots.parameters || self.tail.is_some() || self.contextual_entry.is_some()
            || self.projected_slot.is_some() || self.projected_pair.is_some() || !is_integer_type(ty)
            || !super::typecheck::integer_literal_fits_type(&entry, ty)
        {
            return None;
        }
        let [row] = self.block.path.0.as_slice() else { return None; };
        if row.len() != axis || row.affine.is_some() || row.entry.is_some()
            || (axis == 1 && row.get(0)?.singleton_value().is_none()) { return None; }
        self.block.path.append_local(IntegerSet::singleton(&entry));
        self.roots.add(binding, ty);
        self.block.depth = self.roots.axes.len();
        self.contextual_entry = Some((axis, entry));
        self.roots.allow_exact_contextual_counter_argument = axis == 1;
        self.roots.allow_exact_contextual_resource_guard = axis == 1 && guarded_resource;
        Some(())
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn resume_exact_local_update(
        &mut self, binding: &ReferenceBindingId, ty: &ScalarType, entry: BigInt, exit: BigInt,
    ) -> Option<()> {
        self.resume_exact_local_updates(std::iter::once(LocalPointeeTransition {
            binding: binding.clone(), ty: ty.clone(), entry, exit,
        }))
    }

    #[cfg(feature = "internal-checker")]
    pub(super) fn resume_exact_local_updates(
        &mut self, updates: impl ExactSizeIterator<Item = LocalPointeeTransition>,
    ) -> Option<()> {
        if !(1..=2).contains(&updates.len()) { return None; }
        let mut pending: [Option<(usize, Option<BigInt>)>; 2] = [None, None];
        for (index, update) in updates.enumerate() {
            if self.contextual_singleton(&update.binding, &update.ty)? != &update.entry
                || !super::typecheck::integer_literal_fits_type(&update.exit, &update.ty)
            {
                return None;
            }
            let axis = self.roots.lookup(&update.binding)?.0;
            if pending.iter().flatten().any(|&(used, _)| used == axis) { return None; }
            pending[index] = Some((axis, (update.entry != update.exit).then_some(update.exit)));
        }
        let charges = pending.iter().flatten().filter(|(_, exit)| exit.is_some()).count();
        if charges > 0 {
            self.roots.ordinary_local_work.get_or_insert(0);
            self.block.path.charge_local_alias_work(&mut self.roots.ordinary_local_work, charges.checked_mul(2)?)?;
            let [row] = self.block.path.0.as_mut_slice() else { return None; };
            for (axis, exit) in pending.into_iter().flatten() {
                if let Some(exit) = exit { row.overwrite(axis, Rc::new(IntegerSet::singleton(&exit)), false); }
            }
        }
        Some(())
    }

    pub(super) fn arguments<'borrow>(
        &'borrow mut self, calls: &'borrow FunctionCallSites, call: OccurrenceOrdinal, parameters: usize,
        collect: bool, project: bool, project_pair: bool, project_independent: bool, allow_bool: bool,
    ) -> Option<NestedIntegerArgument<'borrow>> {
        let mut work = self.roots.affine_work;
        let ordinary_arguments = {
            #[cfg(feature = "internal-checker")]
            if let Some(receipt) = calls.structure.checker_ordinary_directive_calls.get(&call) {
                let signature = calls.structure.checker_directives.get(&call)?.call_signature.as_ref()?;
                Some(ordinary_directive_arguments(&receipt.operands, &signature.parameters, &self.roots,
                    &self.block.path, project, project_pair, work.as_mut())?)
            } else { None }
            #[cfg(not(feature = "internal-checker"))]
            { None }
        };
        let mut argument = if let Some(arguments) = ordinary_arguments {
            arguments
        } else if collect && parameters == 2 {
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
            #[cfg(feature = "internal-checker")]
            (ExactCallEffect::TailInteger { ty, .. }, ActualCallResult::PartitionedInteger(pieces))
                if caller_axis.is_some() && self.tail == Some(ty) && self.roots.allow_real_integer_tail_call
                    && self.discards_symbolic_integer_return() && self.tail_call_result.is_none()
                    && !pieces.is_empty() =>
            {
                self.tail_call_result = Some(ActualCallResult::Unit);
            }
            #[cfg(feature = "internal-checker")]
            (ExactCallEffect::TailInteger { ty, .. }, result @ (ActualCallResult::Integer(_) | ActualCallResult::EmptyScalar)) => {
                if caller_axis.is_some() || self.tail != Some(ty) || !self.roots.allow_real_integer_tail_call
                    || self.tail_call_result.is_some()
                {
                    return None;
                }
                self.tail_call_result = Some(result);
            }
            (ExactCallEffect::Unit(_), ActualCallResult::Unit) => {}
            (ExactCallEffect::Unit(_), ActualCallResult::EmptyUnit) => self.block.path = IntegerInputs::default(),
            #[cfg(feature = "internal-checker")]
            (ExactCallEffect::Scalar { binding: Some(_), ty, .. }, ActualCallResult::EmptyScalar) if is_integer_type(ty) || *ty == ScalarType::Bool =>
                self.block.path = IntegerInputs::default(),
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
                let [first, second] = projected.caller_axes;
                let types = [self.roots.axes.get(first)?.1, self.roots.axes.get(second)?.1];
                let axes = [Some(first), Some(second)];
                for piece in projected.pieces {
                    let index = usize::from(piece.anchor).checked_sub(1)?;
                    let row = original.get(index)?;
                    uses_affine |= attach_pair_return(row, axes, types, ty, piece.inputs, &piece.output, &mut joined, &mut work)?;
                }
                if uses_affine { self.roots.affine_work = Some(work); }
                self.finish_scalar_rows(joined, binding, ty)?;
            }
            (ExactCallEffect::Scalar { binding, ty, .. }, ActualCallResult::PartitionedInteger(pieces)) => {
                let axis = caller_axis?;
                let mut retained = self.roots.affine_work;
                let mut joined = self.scalar_output_rows(binding, ty)?;
                transfer_unary_return_partitions(&self.block.path.0, axis, self.roots.axes.get(axis)?.1, ty,
                    self.roots.axes.len().checked_add(1)?, &pieces, &mut joined, &mut retained, None)?;
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
        if let Some(result) = self.tail_call_result.take() {
            return Some(ExactCallProgress::Done { active: self.roots.reached_obligation, result });
        }
        #[cfg(feature = "internal-checker")]
        {
            self.roots.allow_ordinary_integer_locals = allow_ordinary_integer_locals && !calls.structure.checker_ordinary_scalar_locals.is_empty();
            self.roots.allow_ordinary_affine_overwrites = allow_ordinary_integer_locals;
            self.roots.allow_ordinary_contextual_places =
                allow_ordinary_integer_locals && calls.structure.has_ordinary_contextual_places();
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
        #[cfg(feature = "internal-checker")]
        let (allow_ordinary_computed_returns, serialize_affine) = (
            allow_ordinary_computed_returns || self.named_pure_projection_leaf,
            allow_ordinary_computed_returns || self.named_affine_partition || self.roots.named_pair_rectangles,
        );
        #[cfg(not(feature = "internal-checker"))]
        let serialize_affine = allow_ordinary_computed_returns;
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
            #[cfg(feature = "internal-checker")]
            (None, IntegerControl::Continue(path)) if self.contextual_entry.is_some() => {
                let (axis, entry) = self.contextual_entry.take()?;
                let [row] = path.0.as_slice() else { return None; };
                let exit = row.get(axis)?.singleton_value()?.clone();
                ActualCallResult::ContextualUnit { entry, exit }
            }
            #[cfg(feature = "internal-checker")]
            (None, IntegerControl::Returned) if self.contextual_entry.is_some() => return None,
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
                        #[cfg(feature = "internal-checker")]
                        if self.roots.named_pair_rectangles {
                            row.require_named_pair_entry(self.roots.named_pair_bool_axis, &mut self.return_capture_work)?;
                        }
                        let output = if let Some(axis) = alias {
                            if row[axis].singleton_value().is_some() {
                                IntegerPairPartitionOutput::Singleton(Rc::clone(&row[axis]))
                            } else if let Some(slot) = (0..2).find(|slot| row.entry_alias(*slot, axis)) {
                                IntegerPairPartitionOutput::InputAlias(slot)
                            } else if serialize_affine {
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
                            #[cfg(feature = "internal-checker")]
                            span: path.span,
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
                            } else if serialize_affine {
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
            #[cfg(feature = "internal-checker")]
            (Some(_), IntegerControl::Continue(path)) if COLLECT && self.allow_empty_scalar_return && path.is_unreachable() =>
                ActualCallResult::EmptyScalar,
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
fn checked_snapshot_operation<'a>(
    roots: &IntegerRoots<'_>, ty: &ScalarType, source: &ReferenceBindingId, operator: &BinaryOperator,
    literal: &'a BigInt, source_on_left: bool,
) -> Option<(AuthoredIntegerOperation, IntegerCalculationOperand<'a>, IntegerCalculationOperand<'a>)> {
    let (axis, actual) = roots.lookup(source)?;
    if actual != ty { return None; }
    let source = (None, Some(axis));
    let literal = (Some(literal), None);
    let (left, right) = if source_on_left { (source, literal) } else { (literal, source) };
    Some((AuthoredIntegerOperation::checked(operator)?, left, right))
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
            let value = operation.apply(Cow::Borrowed(left), Cow::Borrowed(right), min, max)?;
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
#[derive(Clone, Copy)]
enum FiniteIntegerInstall {
    Append,
    OverwriteSource,
}

#[cfg(feature = "internal-checker")]
impl IntegerInputs {
    fn finite_ordinary_calculation(
        self, operation: AuthoredIntegerOperation, left: IntegerCalculationOperand<'_>,
        right: IntegerCalculationOperand<'_>, ty: &ScalarType,
        install: FiniteIntegerInstall, work: &mut usize,
    ) -> Option<Self> {
        // Enumerate a small complete preimage, not a sample or a convex hull.
        const MAX_FINITE_ARITHMETIC_ATOMS: usize = 64;
        let (axis, literal, lexical_on_left) = match (left, right) {
            ((None, Some(axis)), (Some(literal), None)) => (axis, literal, true),
            ((Some(literal), None), (None, Some(axis))) => (axis, literal, false),
            _ => return None,
        };
        let (min, max) = super::typecheck::integer_bounds(ty)?;
        let one = BigInt::from(1u8);
        let mut remaining = MAX_FINITE_ARITHMETIC_ATOMS;
        let mut output = Vec::new();
        for row in self.0 {
            if row.affine.is_some() || row.entry.is_some() { return None; }
            let domain = row.get(axis)?;
            charge_call_work(work, 1usize.checked_add(domain.interval_count())?)?;
            let alias_passes = match install { FiniteIntegerInstall::Append => 3, FiniteIntegerInstall::OverwriteSource => 7 };
            let copy_cost = row.len().checked_mul(2)?
                .checked_add(row.equal_axes().len().checked_mul(alias_passes)?)?.checked_add(10)?;
            for (first, last) in domain.intervals() {
                let count = usize::try_from(last - first + &one).ok()?;
                remaining = remaining.checked_sub(count)?;
                charge_call_work(work, copy_cost.checked_mul(count)?)?;
                let mut input = first.clone();
                for index in 0..count {
                    let (left, right) = if lexical_on_left { (&input, literal) } else { (literal, &input) };
                    let value = operation.apply(Cow::Borrowed(left), Cow::Borrowed(right), &min, &max)?;
                    if value < min || value > max { return None; }
                    let mut piece = row.clone();
                    // Refining the old source preserves all of its aliases;
                    // the computed value belongs only to this exact preimage.
                    piece.replace_domain(axis, Rc::new(IntegerSet::singleton(&input)));
                    let value = Rc::new(IntegerSet::singleton(&value));
                    match install {
                        FiniteIntegerInstall::Append => piece.push(value),
                        FiniteIntegerInstall::OverwriteSource => {
                            // Detach only this binding. Previously read
                            // aliases retain the exact old preimage.
                            piece.overwrite(axis, value, false);
                        }
                    }
                    output.push(piece);
                    if index + 1 < count { input += &one; }
                }
            }
        }
        Some(Self(output))
    }

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

#[cfg(feature = "internal-checker")]
enum ParallelScalarSource {
    Copy(usize),
    Literal(Rc<IntegerSet>),
}

#[cfg(feature = "internal-checker")]
fn ordinary_parallel_assignment(
    calls: &FunctionCallSites, roots: &mut IntegerRoots<'_>, position: BlockPosition, state: IntegerInputs,
) -> Option<IntegerInputs> {
    const MAX_PARALLEL_SCALARS: usize = 16;
    let group = calls.assignments.get(&(position.block, position.item))?;
    let count = group.destinations().len();
    if !(2..=MAX_PARALLEL_SCALARS).contains(&count) || group.rhs_evaluations().len() != count
        || group.projected_outputs().len() != 0 || roots.projected_entry_axis.is_some()
    {
        return None;
    }
    let structure = &calls.structure;
    let width = roots.axes.len();
    let mut work = roots.ordinary_local_work.take().unwrap_or_default();
    charge_call_work(&mut work, width.checked_add(count.checked_mul(8)?)?)?;
    let mut changes = (0..width).map(|_| None).collect::<Vec<_>>();
    for (index, (destination, rhs)) in group.destinations().zip(group.rhs_evaluations()).enumerate() {
        if destination.position != index || rhs.position != index
            || structure.checker_contextual_occurrences.contains(&destination.destination)
            || structure.checker_contextual_occurrences.contains(&rhs.expression)
        {
            return None;
        }
        let DirectDestinationIdentity::Existing(binding) = calls.prepared_direct_destination(destination.destination)?
            else { return None; };
        let (axis, ty) = roots.lookup(binding)?;
        if axis < roots.parameters || changes[axis].is_some()
            || (!is_integer_type(ty) && *ty != ScalarType::Bool)
            || !matches!(&destination.replacement, AssignmentReplacement::Resolved(resolved) if resolved == ty)
            || structure.checked_types.expressions.get(&rhs.expression)? != ty
        {
            return None;
        }
        let operand = if let Some(value) = scalar_call_literal(structure, rhs.expression, ty, true) {
            ParallelScalarSource::Literal(Rc::new(IntegerSet::singleton(value)))
        } else {
            let PlainNameIdentity::Lexical(source) = calls.prepared_plain_name(rhs.expression)? else { return None; };
            let (source, source_ty) = roots.lookup(source)?;
            if source_ty != ty { return None; }
            ParallelScalarSource::Copy(source)
        };
        changes[axis] = Some(operand);
    }
    let state = state.parallel_overwrite(&changes, &mut work)?;
    roots.ordinary_local_work = Some(work);
    Some(state)
}

#[cfg(feature = "internal-checker")]
impl IntegerInputs {
    fn parallel_overwrite(mut self, changes: &[Option<ParallelScalarSource>], work: &mut usize) -> Option<Self> {
        for row in &mut self.0 {
            if row.affine.is_some() || row.entry.is_some() || row.len() != changes.len() { return None; }
            let levels = (usize::BITS - row.equal_axes().len().leading_zeros()) as usize;
            let width = row.len();
            charge_call_work(work, 1usize.checked_add(width.checked_mul(5usize.checked_add(levels)?)?)?)?;
            // Build all new cells and equality classes from the complete
            // pre-write row. No destination changes until all RHS origins
            // for this row (including reused old destinations) are resolved.
            let mut representatives = vec![None; width];
            let mut replacement = IntegerRow::new(Vec::with_capacity(width));
            replacement.projected_from = row.projected_from;
            let mut aliases = Vec::new();
            for (axis, source) in changes.iter().enumerate() {
                let old = match source {
                    Some(ParallelScalarSource::Literal(value)) => {
                        replacement.cells.push(value.clone());
                        continue;
                    }
                    Some(ParallelScalarSource::Copy(source)) => *source,
                    None => axis,
                };
                let origin = row.canonical_axis(old);
                let domain = Rc::clone(&row[old]);
                if let Some(representative) = representatives[origin] {
                    aliases.push((axis, representative));
                } else {
                    representatives[origin] = Some(axis);
                }
                replacement.cells.push(domain);
            }
            replacement.equal_axes = (!aliases.is_empty()).then(|| Rc::new(aliases));
            *row = replacement;
        }
        Some(self)
    }
}

#[cfg(feature = "internal-checker")]
struct ProjectedScalarLocalWrite<'a> {
    destination: &'a ReferenceBindingId,
    rhs: OccurrenceOrdinal,
    ty: &'a ScalarType,
}

#[cfg(feature = "internal-checker")]
fn projected_scalar_local_write(calls: &FunctionCallSites, position: BlockPosition) -> Option<ProjectedScalarLocalWrite<'_>> {
    let (destination, ty, rhs, place) = checked_scalar_assignment(calls, position)?;
    let structure = &calls.structure;
    if !is_integer_type(ty) && *ty != ScalarType::Bool
        || structure.checked_types.expressions.get(&rhs)? != ty
        || structure.checker_contextual_occurrences.contains(&place)
        || structure.checker_contextual_occurrences.contains(&rhs)
    {
        return None;
    }
    // Literal-only writes still use the preexisting assignment path.
    (matches!(calls.prepared_plain_name(rhs), Some(PlainNameIdentity::Lexical(_)))
        || matches!(calls.conditionals.get(&rhs), Some(ConditionalRegion::Value { .. })))
        .then_some(ProjectedScalarLocalWrite { destination, rhs, ty })
}

#[cfg(feature = "internal-checker")]
fn apply_projected_scalar_local_write(
    calls: &FunctionCallSites, roots: &mut IntegerRoots<'_>, path: IntegerInputs, write: ProjectedScalarLocalWrite<'_>,
) -> Option<IntegerInputs> {
    let (destination, destination_ty) = roots.lookup(write.destination)?;
    if destination < roots.parameters || destination_ty != write.ty { return None; }
    let structure = &calls.structure;
    let operand = |roots: &IntegerRoots<'_>, expression, literal| {
        if structure.checked_types.expressions.get(&expression)? != write.ty
            || structure.checker_contextual_occurrences.contains(&expression)
        {
            return None;
        }
        if literal {
            if let Some(value) = scalar_call_literal(structure, expression, write.ty, true) {
                return Some(ParallelScalarSource::Literal(Rc::new(IntegerSet::singleton(value))));
            }
        }
        let PlainNameIdentity::Lexical(source) = calls.prepared_plain_name(expression)? else { return None; };
        let (axis, ty) = roots.lookup(source)?;
        (ty == write.ty).then_some(ParallelScalarSource::Copy(axis))
    };
    let overwrite = |roots: &mut IntegerRoots<'_>, path: IntegerInputs, value| {
        let width = roots.axes.len();
        let work = roots.ordinary_local_work.get_or_insert(0);
        charge_call_work(work, width.checked_add(1)?)?;
        let mut changes = (0..width).map(|_| None).collect::<Vec<_>>();
        changes[destination] = Some(value);
        // Rebuild equality using the old coordinates; the unchanged formal
        // entry and the destination's prior aliases retain their own domains.
        path.parallel_overwrite(&changes, work)
    };
    if !matches!(calls.conditionals.get(&write.rhs), Some(ConditionalRegion::Value { .. })) {
        let value = operand(roots, write.rhs, false)?;
        return overwrite(roots, path, value);
    }

    let lanes = ordinary_scalar_conditional_lanes(calls, roots, path, write.rhs, false)?;
    let mut joined = IntegerInputs::default();
    for (arm, block) in lanes {
        if arm.is_unreachable() { continue; }
        let leaf = checked_conditional_scalar_leaf(structure, block)?;
        let value = operand(roots, leaf, true)?;
        let mut updated = overwrite(roots, arm, value)?;
        charge_call_work(roots.ordinary_local_work.get_or_insert(0), updated.0.len())?;
        joined.0.append(&mut updated.0);
    }
    Some(joined)
}

#[cfg(feature = "internal-checker")]
fn ordinary_scalar_conditional_lanes(
    calls: &FunctionCallSites, roots: &mut IntegerRoots<'_>, path: IntegerInputs, source: OccurrenceOrdinal,
    fresh_binding: bool,
) -> Option<[(IntegerInputs, BlockOrdinal); 2]> {
    let ConditionalRegion::Value { condition, then_block, else_block, resume, .. } =
        calls.conditionals.get(&source)? else { return None; };
    let structure = &calls.structure;
    charge_call_work(roots.ordinary_local_work.get_or_insert(0), 8)?;
    if *resume != source || structure.checked_types.expressions.get(condition)? != &ScalarType::Bool
        || structure.checker_contextual_occurrences.contains(condition)
    {
        return None;
    }
    let predicate = structure.checker_branches.get(condition)?;
    match predicate {
        CheckerPredicate::Boolean(_) => {}
        CheckerPredicate::Integer(_) => plain_conditional_integer_operands(structure, *condition)?,
        _ => return None,
    }
    // Detached entries are outside both transfer profiles. The overwrite
    // path retains its existing ban on any affine relation.
    charge_call_work(roots.ordinary_local_work.get_or_insert(0), path.0.len())?;
    let mut affine = false;
    for row in &path.0 {
        if row.entry.is_some() { return None; }
        affine |= row.affine.is_some();
    }
    if affine {
        if !fresh_binding { return None; }
        // Fresh storage only appends an axis. The fully metered cut
        // propagates a restriction to linked members before either arm reads.
        let work = roots.ordinary_local_work.get_or_insert(0);
        return match predicate {
            CheckerPredicate::Boolean(true) => Some([(path, *then_block), (IntegerInputs::default(), *else_block)]),
            CheckerPredicate::Boolean(false) => Some([(IntegerInputs::default(), *then_block), (path, *else_block)]),
            CheckerPredicate::Integer(predicate) => {
                let axis = *roots.by_binding.get(&predicate.binding)?;
                if roots.axes.get(axis)?.1 != &predicate.ty { return None; }
                let relation = IntegerRelation::from_operator(&predicate.operator)?;
                let [failing, passing] = path.ordinary_atom(
                    OrdinaryIntegerAtom::Literal(axis, relation, &predicate.bound), [true, true], work,
                )?;
                Some([(passing, *then_block), (failing, *else_block)])
            }
            _ => None,
        };
    }
    path.charge_projected_integer_directive(roots, predicate)?;
    let (passing, failing, _) = path.split(roots, predicate)?;
    Some([(passing, *then_block), (failing, *else_block)])
}

#[cfg(feature = "internal-checker")]
fn bind_enum_match_value(
    roots: &mut IntegerRoots<'_>, path: IntegerInputs,
    transfer: &super::runtime_enum_match::MatchTransfer<ReferenceBindingId>, ty: &ScalarType,
) -> Option<IntegerInputs> {
    use super::runtime_enum_match::{MatchAddend, MatchCondition, MatchLane, MatchTransfer, MatchValue};
    if !is_integer_type(ty) { return None; }
    charge_call_work(roots.ordinary_local_work.get_or_insert(0), path.0.len().checked_add(1)?)?;
    if path.0.iter().any(|row| row.affine.is_some() || row.entry.is_some()) { return None; }

    let install = |roots: &mut IntegerRoots<'_>, mut lane: IntegerInputs, value: &MatchValue<ReferenceBindingId>| {
        if lane.is_unreachable() { return Some(lane); }
        charge_ordinary_scalar_append(roots, &lane)?;
        match value {
            MatchValue::Literal(value) => lane.append_local(IntegerSet::singleton(value)),
            MatchValue::Copy(binding) => {
                let (axis, source_ty) = roots.lookup(binding)?;
                if source_ty != ty { return None; }
                lane.append_ordinary_copy(axis, roots)?;
            }
            MatchValue::WrappingAdd { source, offset } => {
                let operation = AuthoredIntegerOperation::WrappingAdd;
                let right = (Some(offset), None);
                let left = match source {
                    MatchAddend::Literal(value) => (Some(value), None),
                    MatchAddend::Copy(binding) => {
                        let (axis, source_ty) = roots.lookup(binding)?;
                        if source_ty != ty { return None; }
                        charge_call_work(roots.ordinary_local_work.get_or_insert(0), lane.0.len())?;
                        if lane.0.iter().any(|row| row.get(axis).is_none_or(|domain| domain.singleton_value().is_none())) {
                            return append_wrapped_match_axis(roots, lane, axis, ty, offset);
                        }
                        (None, Some(axis))
                    }
                };
                lane.append_ordinary_calculation(operation, left, right, ty,
                    &mut roots.ordinary_local_work, &mut roots.affine_work)?;
            }
        }
        Some(lane)
    };
    let install_lane = |roots: &mut IntegerRoots<'_>, mut rows: IntegerInputs, lane: &MatchLane<ReferenceBindingId>, guard| {
        if let Some(history) = roots.callable_history {
            history.borrow_mut().append_match_lane(&mut rows, guard, lane.arm_scope)?;
        }
        install(roots, rows, &lane.output)
    };
    match transfer {
        MatchTransfer::Value { lane, guard } => install_lane(roots, path, lane, *guard),
        MatchTransfer::Select { condition_span, condition, yes, no } => {
            // Keep the selector coordinate and its equalities on every cut row.
            let one = BigInt::from(1);
            let (binding, ty, relation, bound) = match condition {
                MatchCondition::Boolean(binding) => (binding, &ScalarType::Bool,
                    IntegerRelation::from_operator(&BinaryOperator::Equal)?, &one),
                MatchCondition::Integer { binding, ty, relation, bound } if is_integer_type(ty) =>
                    (binding, ty, *relation, bound),
                MatchCondition::Integer { .. } => return None,
            };
            let (axis, checked_ty) = roots.lookup(binding)?;
            if checked_ty != ty { return None; }
            let [no_path, yes_path] = path.ordinary_atom(
                OrdinaryIntegerAtom::Literal(axis, relation, bound),
                [true, true], roots.ordinary_local_work.get_or_insert(0),
            )?;
            let mut joined = install_lane(roots, yes_path, yes, Some((*condition_span, true)))?;
            let mut no_path = install_lane(roots, no_path, no, Some((*condition_span, false)))?;
            append_ordinary_scalar_lane(roots, &mut joined, &mut no_path)?;
            Some(joined)
        }
    }
}

/// A fitted fixed-width addend can cross one endpoint at most. Retain
/// each exact preimage row, then append its corresponding affine result.
#[cfg(feature = "internal-checker")]
fn append_wrapped_match_axis(
    roots: &mut IntegerRoots<'_>, path: IntegerInputs, axis: usize, ty: &ScalarType, offset: &BigInt,
) -> Option<IntegerInputs> {
    use num_bigint::Sign;
    let (min, max) = super::typecheck::integer_bounds(ty)?;
    if offset < &min || offset > &max { return None; }
    if offset.sign() == Sign::NoSign {
        let mut path = path;
        path.append_ordinary_copy(axis, roots)?;
        return Some(path);
    }
    let width = &max - &min + BigInt::from(1);
    let (bound, comparison, wrapped) = if offset.sign() == Sign::Plus {
        (&max - offset, BinaryOperator::LessEqual, offset - &width)
    } else {
        (&min - offset, BinaryOperator::GreaterEqual, offset + &width)
    };
    let relation = IntegerRelation::from_operator(&comparison)?;
    let [wrapped_path, plain_path] = path.ordinary_atom(
        OrdinaryIntegerAtom::Literal(axis, relation, &bound), [true, true],
        roots.ordinary_local_work.get_or_insert(0),
    )?;
    let mut joined = IntegerInputs::default();
    for (mut lane, offset) in [(plain_path, offset.clone()), (wrapped_path, wrapped)] {
        if lane.is_unreachable() { continue; }
        charge_ordinary_scalar_append(roots, &lane)?;
        let transform = Rc::new(IntegerAffineTransform { reflected: false, offset });
        let work = roots.ordinary_local_work.get_or_insert(0);
        for row in &mut lane.0 {
            let domain = row.get(axis)?;
            // The cut establishes fit; recheck with the canonical affine
            // domain contract before any piece can survive.
            transform.fits(domain, &min, &max)?;
            let plan = row.prepare_affine(axis, domain, &transform, work)?;
            if matches!(&plan, AffineAppend::Member { .. }) {
                roots.affine_work.get_or_insert(0);
            }
            row.append_affine(plan);
        }
        append_ordinary_scalar_lane(roots, &mut joined, &mut lane)?;
    }
    Some(joined)
}

#[cfg(feature = "internal-checker")]
fn charge_ordinary_scalar_append(roots: &mut IntegerRoots<'_>, path: &IntegerInputs) -> Option<()> {
    // Appending fresh storage can grow the entire row cell vector.
    let work = roots.ordinary_local_work.get_or_insert(0);
    for row in &path.0 { charge_call_work(work, row.len().checked_add(1)?)?; }
    Some(())
}

#[cfg(feature = "internal-checker")]
fn append_ordinary_scalar_lane(
    roots: &mut IntegerRoots<'_>, joined: &mut IntegerInputs, lane: &mut IntegerInputs,
) -> Option<()> {
    let total = joined.0.len().checked_add(lane.0.len())?;
    let relocated = if total > joined.0.capacity() { joined.0.len() } else { 0 };
    charge_call_work(roots.ordinary_local_work.get_or_insert(0), lane.0.len().checked_add(relocated)?)?;
    joined.0.append(&mut lane.0);
    Some(())
}

#[cfg(feature = "internal-checker")]
fn bind_conditional_scalar_local(
    calls: &FunctionCallSites, roots: &mut IntegerRoots<'_>, path: IntegerInputs,
    source: OccurrenceOrdinal, ty: &ScalarType,
) -> Option<IntegerInputs> {
    let structure = &calls.structure;
    let mut joined = IntegerInputs::default();
    for (mut arm, block) in ordinary_scalar_conditional_lanes(calls, roots, path, source, true)? {
        if arm.is_unreachable() { continue; }
        let leaf = checked_conditional_scalar_leaf(structure, block)?;
        if structure.checked_types.expressions.get(&leaf)? != ty
            || structure.checker_contextual_occurrences.contains(&leaf)
        {
            return None;
        }
        charge_ordinary_scalar_append(roots, &arm)?;
        if let Some(value) = scalar_call_literal(structure, leaf, ty, true) {
            arm.append_local(IntegerSet::singleton(value));
        } else {
            let PlainNameIdentity::Lexical(binding) = calls.prepared_plain_name(leaf)? else { return None; };
            let (axis, source_ty) = roots.lookup(binding)?;
            if source_ty != ty { return None; }
            arm.append_ordinary_copy(axis, roots)?;
        }
        append_ordinary_scalar_lane(roots, &mut joined, &mut arm)?;
    }
    Some(joined)
}

#[cfg(feature = "internal-checker")]
fn plain_conditional_integer_operands(structure: &OccurrenceStructure, condition: OccurrenceOrdinal) -> Option<()> {
    for edge in [OccurrenceEdge::BinaryLeft, OccurrenceEdge::BinaryRight] {
        let OccurrenceNode::Expression(operand) = structure.edges.get(&(
            OccurrenceParent::Expression(condition), edge,
        ))? else { return None; };
        if structure.checker_contextual_occurrences.contains(operand) { return None; }
    }
    Some(())
}

#[cfg(feature = "internal-checker")]
fn checked_conditional_scalar_leaf(structure: &OccurrenceStructure, block: BlockOrdinal) -> Option<OccurrenceOrdinal> {
    let block = structure.blocks.get(block.0)?;
    let [item] = block.items.as_slice() else { return None; };
    let BlockItemKind::FinalOutput(slot) = &item.kind else { return None; };
    (slot.output == 0 && block.final_outputs().nth(1).is_none()).then_some(slot.expression)
}

#[cfg(feature = "internal-checker")]
fn exact_returning_local_write(
    calls: &FunctionCallSites, roots: &mut IntegerRoots<'_>, position: BlockPosition, mut path: IntegerInputs,
) -> Option<IntegerInputs> {
    let (binding, replacement, rhs, destination) = checked_scalar_assignment(calls, position)?;
    let (axis, ty) = roots.lookup(binding)?;
    let structure = &calls.structure;
    if axis < roots.parameters || ty != replacement || !is_integer_type(ty) && *ty != ScalarType::Bool
        || structure.checked_types.expressions.get(&rhs)? != ty
        || structure.checker_contextual_occurrences.contains(&destination)
        || structure.checker_contextual_occurrences.contains(&rhs)
    {
        return None;
    }
    // This exact startup lane owns a single concrete row. Never fall
    // through to the general overwrite path's multi-row compaction.
    let [row] = path.0.as_mut_slice() else { return None; };
    if row.affine.is_some() || row.entry.is_some() { return None; }
    row.get(axis)?.singleton_value()?;
    let work = roots.ordinary_local_work.get_or_insert(0);
    charge_call_work(work, 4usize.checked_add(row.equal_axes().len().checked_mul(3)?)?)?;
    let value = exact_returning_local_write_value(calls, roots, row, rhs, ty)?;
    row.overwrite(axis, value, false);
    Some(path)
}

#[cfg(feature = "internal-checker")]
fn exact_returning_local_write_value(
    calls: &FunctionCallSites, roots: &mut IntegerRoots<'_>, row: &IntegerRow,
    rhs: OccurrenceOrdinal, ty: &ScalarType,
) -> Option<Rc<IntegerSet>> {
    use super::reference::CheckerOrdinaryBoolean;
    let structure = &calls.structure;
    let selected = if let Some(ConditionalRegion::Value { condition, then_block, else_block, resume, .. }) =
        calls.conditionals.get(&rhs)
    {
        charge_call_work(roots.ordinary_local_work.get_or_insert(0), 8)?;
        if *resume != rhs || structure.checked_types.expressions.get(condition)? != &ScalarType::Bool
            || structure.checker_contextual_occurrences.contains(condition)
        {
            return None;
        }
        let choose = match structure.checker_branches.get(condition) {
            Some(CheckerPredicate::Boolean(value)) => *value,
            Some(CheckerPredicate::Integer(predicate)) => {
                plain_conditional_integer_operands(structure, *condition)?;
                let (axis, axis_ty) = roots.lookup(&predicate.binding)?;
                if axis_ty != &predicate.ty { return None; }
                let value = row.get(axis)?.singleton_value()?;
                IntegerRelation::from_operator(&predicate.operator)?.accepts(value.cmp(&predicate.bound))
            }
            None => {
                let CheckerOrdinaryBoolean::Binding(binding) = structure.checker_ordinary_booleans.get(condition)?
                    else { return None; };
                let (axis, axis_ty) = roots.lookup(binding)?;
                if *axis_ty != ScalarType::Bool { return None; }
                bool_result_lane(row.get(axis)?.singleton_value()?)? == 1
            }
            _ => return None,
        };
        checked_conditional_scalar_leaf(structure, if choose { *then_block } else { *else_block })?
    } else {
        rhs
    };
    if structure.checked_types.expressions.get(&selected)? != ty
        || structure.checker_contextual_occurrences.contains(&selected)
    {
        return None;
    }
    if let Some(value) = scalar_call_literal(structure, selected, ty, true) {
        return Some(Rc::new(IntegerSet::singleton(value)));
    }
    let PlainNameIdentity::Lexical(source) = calls.prepared_plain_name(selected)? else { return None; };
    let (source, source_ty) = roots.lookup(source)?;
    if source_ty != ty { return None; }
    let value = row.get(source)?;
    value.singleton_value()?;
    Some(Rc::clone(value))
}

#[cfg(feature = "internal-checker")]
fn contextual_boolean_source(
    calls: &FunctionCallSites, roots: &IntegerRoots<'_>, rhs: OccurrenceOrdinal,
) -> Option<ParallelScalarSource> {
    if calls.structure.checked_types.expressions.get(&rhs)? != &ScalarType::Bool { return None; }
    if let Some(value) = scalar_call_literal(&calls.structure, rhs, &ScalarType::Bool, true) {
        bool_result_lane(value)?;
        return Some(ParallelScalarSource::Literal(Rc::new(IntegerSet::singleton(value))));
    }
    let PlainNameIdentity::Lexical(source) = calls.prepared_plain_name(rhs)? else { return None; };
    let (axis, ty) = roots.lookup(source)?;
    (*ty == ScalarType::Bool).then_some(ParallelScalarSource::Copy(axis))
}

#[cfg(feature = "internal-checker")]
fn exact_contextual_boolean_value(
    calls: &FunctionCallSites, roots: &IntegerRoots<'_>, row: &IntegerRow, rhs: OccurrenceOrdinal,
) -> Option<Rc<IntegerSet>> {
    match contextual_boolean_source(calls, roots, rhs)? {
        ParallelScalarSource::Literal(value) => Some(value),
        ParallelScalarSource::Copy(axis) => {
            let value = row.get(axis)?;
            bool_result_lane(value.singleton_value()?)?;
            Some(Rc::clone(value))
        }
    }
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

pub(super) fn checked_scalar_assignment(
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
pub(super) fn exact_contextual_summary_leaf(calls: &FunctionCallSites) -> bool {
    fn eligible(calls: &FunctionCallSites) -> Option<()> {
        let (formal, ty) = calls.contextual_formal()?;
        let structure = &calls.structure;
        if !matches!(calls.boundary.results.as_slice(), [out] if out.ty == ScalarType::Unit)
            || structure.blocks.len() != 1 || calls.assignments.len() != 1
            || !calls.arms.is_empty() || !calls.call_occurrences.is_empty()
            || !calls.allocation_length_calls.is_empty() || !structure.checker_directives.is_empty()
        {
            return None;
        }
        let [item] = structure.blocks.get(structure.root.0)?.items.as_slice() else { return None; };
        let BlockItemKind::Assignment { group } = item.kind else { return None; };
        let update = structure.checker_contextual_counter_updates.get(&(group.block, group.item))?;
        let (binding, target_type, rhs, destination) = checked_scalar_assignment(calls, group)?;
        let amount_matches = match (calls.boundary.parameters.as_slice(), &update.amount) {
            ([], super::reference::CheckerContextualCounterOperand::Literal(_)) => true,
            ([parameter], super::reference::CheckerContextualCounterOperand::Lexical(binding)) => {
                parameter.ordinal == 0 && &parameter.ty == ty
                    && matches!(calls.boundary.checker_parameter_bindings.as_deref(), Some([formal]) if formal == binding)
                    && !structure.blocks[structure.root.0].unsafe_context
            }
            _ => false,
        };
        (binding == formal && target_type == ty && rhs == update.rhs
            && matches!(update.operator, BinaryOperator::Add | BinaryOperator::Subtract)
            && amount_matches
            && structure.checker_contextual_occurrences.contains(&destination)).then_some(())
    }
    eligible(calls).is_some()
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


#[cfg(feature = "internal-checker")]
#[test]
fn nominal_hidden_affine_anchor_reanchors_to_exact_visible_endpoints() {
    let domain = IntegerSet::from_type(&ScalarType::I8).unwrap()
        .keep(&BinaryOperator::GreaterEqual, &1.into()).unwrap()
        .keep(&BinaryOperator::LessEqual, &3.into()).unwrap();
    let mut row = IntegerRow::new(vec![Rc::new(domain)]);
    row.test_append_affine(0, Rc::new(IntegerAffineTransform { reflected: true, offset: 6.into() })).unwrap();
    row.test_append_affine(0, Rc::new(IntegerAffineTransform { reflected: false, offset: 1.into() })).unwrap();
    let mut state = IntegerInputs(vec![row]);
    state.remove_snapshot_axes(&[None, Some(0), Some(1)], &mut Vec::new(), &mut 0).unwrap();
    assert!(state.includes_signed(&[5, 2]));
    assert!(state.includes_signed(&[3, 4]));
    assert!(!state.includes_signed(&[5, 4]));
}

#[cfg(feature = "internal-checker")]
#[test]
fn simultaneous_parallel_rows_keep_old_origins_and_relations() {
    let domain = |lo, hi| Rc::new(IntegerSet::from_type(&ScalarType::I8).unwrap()
        .keep(&BinaryOperator::GreaterEqual, &BigInt::from(lo)).unwrap()
        .keep(&BinaryOperator::LessEqual, &BigInt::from(hi)).unwrap());
    let old_first = domain(1, 3);
    let old_second = domain(7, 9);
    let mut row = IntegerRow::new(vec![old_first.clone(), old_second]);
    row.append_alias(0, old_first);
    let changes = [
        Some(ParallelScalarSource::Copy(1)),
        Some(ParallelScalarSource::Copy(0)),
        None,
    ];
    let mut work = 0;
    let updated = IntegerInputs(vec![row.clone()]).parallel_overwrite(&changes, &mut work).unwrap();
    assert!(updated.includes_signed(&[8, 2, 2]));
    assert!(!updated.includes_signed(&[2, 8, 2]));
    assert!(!updated.includes_signed(&[8, 2, 3]));
    assert_eq!(updated.row_equal_axes(0).unwrap(), [(2, 1)]);
    assert!(work > 0);
    let mut exhausted = MAX_PARTITION_TRANSFER_WORK;
    assert!(IntegerInputs(vec![row]).parallel_overwrite(&changes, &mut exhausted).is_none());
}

#[cfg(feature = "internal-checker")]
#[test]
fn bounded_snapshot_index_preserves_holes_and_affine_preimages_before_aliasing() {
    let singleton = |value: i32| Rc::new(IntegerSet::singleton(&BigInt::from(value)));
    let domain = Rc::new(IntegerSet::union([singleton(0).as_ref(), singleton(2).as_ref()]));
    let mut row = IntegerRow::new(vec![domain, singleton(42), singleton(99)]);
    assert!(row.test_append_affine(0, Rc::new(IntegerAffineTransform { reflected: true, offset: 6.into() })).is_some());
    use super::checker_directive::CheckerSnapshotNode as Node;
    let fields = |index| [0, 1, 3].get(index).copied();
    let array = Node::Array(Box::new([Node::Leaf(0), Node::Leaf(1), Node::Leaf(2)]));
    let steps = [(3, SnapshotIndexValue::Axis(0))];
    let mut work = 0;
    let result = IntegerInputs(vec![row.clone()]).bounded_snapshot_path(&array, &steps, fields, &mut work).unwrap();
    assert_eq!(result.row_count(), 2);
    assert!(result.includes_signed(&[0, 42, 99, 6, 0]));
    assert!(result.includes_signed(&[2, 42, 99, 4, 4]));
    assert!(!result.includes_signed(&[0, 42, 99, 6, 4]));
    assert!(!result.includes_signed(&[1, 42, 99, 5, 42]));
    let shorter = Node::Array(Box::new([Node::Leaf(0), Node::Leaf(1)]));
    assert!(IntegerInputs(vec![row.clone()]).bounded_snapshot_path(&shorter,
        &[(2, SnapshotIndexValue::Axis(0))], fields, &mut 0).is_none());
    let mut almost_exhausted = MAX_PARTITION_TRANSFER_WORK - 1;
    assert!(IntegerInputs(vec![row]).bounded_snapshot_path(&array, &steps, fields, &mut almost_exhausted).is_none());
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
        directive_before: None,
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
    let [false_lane, true_lane] = input.ordinary_boolean(&dead_or, &roots, [true, true], &mut work, None).unwrap();
    assert!(false_lane.is_unreachable());
    assert!(!true_lane.is_unreachable());
    let dead_and = Boolean::And(Box::new([Boolean::Constant(false), Boolean::Opaque]));
    let [false_lane, true_lane] = input.ordinary_boolean(&dead_and, &roots, [true, false], &mut work, None).unwrap();
    assert!(!false_lane.is_unreachable());
    assert!(true_lane.is_unreachable());
    let reached = Boolean::Not(Box::new(Boolean::Or(Box::new([Boolean::Constant(false), Boolean::Opaque]))));
    assert!(input.ordinary_boolean(&reached, &roots, [false, true], &mut work, None).is_none());
    let mut exhausted = MAX_PARTITION_TRANSFER_WORK;
    assert!(input.ordinary_boolean(&dead_or, &roots, [true, true], &mut exhausted, None).is_none());
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
