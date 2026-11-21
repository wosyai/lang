use std::{collections::HashMap, rc::Rc};

use num_bigint::BigInt;

use super::{
    checker_directive::CheckerOperation,
    control::{AssignmentReplacement, BindingInitialization, BindingRegion, BlockItemKind, BlockPosition, ConditionalRegion},
    reference::{BlockOrdinal, CheckerIntegerCallArgument, DirectDestinationIdentity, FunctionCallSites, OccurrenceOrdinal, PlainNameIdentity},
    symbolic_integer::{IntegerRelation, IntegerSet},
    typecheck::is_integer_type,
    BinaryOperator, ReferenceBindingId, ScalarExpression, ScalarType,
};

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
    pub(super) bound: &'a BigInt,
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
        let ScalarExpression::Integer { value: bound, .. } = right.as_ref() else {
            return None;
        };
        Some(Self { left, right, name, operator, bound })
    }

    pub(super) fn typed(
        &self,
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
            bound: self.bound.clone(),
        })
    }
}

pub(super) struct ImportedLocal<'a> {
    pub(super) position: usize,
    pub(super) values: IntegerSet,
    pub(super) guard: Option<(IntegerRelation, &'a BigInt)>,
}

type IntegerRow = Vec<Rc<IntegerSet>>;
#[derive(Default)]
struct IntegerRoots<'a> {
    axes: Vec<(&'a ReferenceBindingId, &'a ScalarType)>,
    by_binding: HashMap<&'a ReferenceBindingId, usize>,
    parameters: usize,
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

    pub(super) fn row_count(&self) -> usize {
        self.0.len()
    }

    pub(super) fn includes(&self, coordinates: &[u128]) -> bool {
        self.0.iter().any(|row| {
            row.len() == coordinates.len()
                && row.iter().zip(coordinates).all(|(axis, value)| {
                    axis.split(&BinaryOperator::Equal, &BigInt::from(*value))
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
    pub(super) axes: Vec<&'a ReferenceBindingId>,
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

pub(super) struct IntegerBranchProof<'a> {
    pub(super) guard: CheckerPredicate,
    pub(super) then_claim: IntegerReturn<'a>,
    pub(super) else_claim: IntegerReturn<'a>,
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

    fn output_satisfies(&self, relation: IntegerRelation, bound: &BigInt) -> Option<bool> {
        match self.value {
            IntegerReturn::Alias(binding) => self.binding_satisfies(binding, relation, bound),
            IntegerReturn::Const(value) => Some(relation.accepts(value.cmp(bound))),
        }
    }

    fn shows_branch(self, proof: &IntegerBranchProof<'_>) -> Option<bool> {
        let (yes, no) = match &proof.guard {
            CheckerPredicate::Boolean(true) => (self.state, IntegerInputs::default()),
            CheckerPredicate::Boolean(false) => (IntegerInputs::default(), self.state),
            CheckerPredicate::Integer(predicate) => {
                let index = self.axes.iter().position(|binding| **binding == predicate.binding)?;
                self.state.split_axis(index, predicate)?
            }
        };
        Some(
            shows_return_equality(&yes, &self.axes, &self.value, &proof.then_claim)?
                && shows_return_equality(&no, &self.axes, &self.value, &proof.else_claim)?,
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

pub(super) fn check_integer_requirements(
    calls: &FunctionCallSites,
) -> Option<Result<(), IntegerCheckFailure<'_>>> {
    run_integer_callable(calls, integer_tail_type(calls), None, None, None)
        .map(|result| result.map(drop))
}

pub(super) fn proves_requirement_with_imported_root_local(
    calls: &FunctionCallSites,
    local: ImportedLocal<'_>,
) -> Option<bool> {
    if local.values.is_empty() {
        return None;
    }
    match run_integer_callable(calls, integer_tail_type(calls), None, None, Some(local))? {
        Ok(ProofCoverage::Applicable(())) => Some(true),
        Ok(ProofCoverage::NoApplicableInputs) => None,
        Err(_) => Some(false),
    }
}

pub(super) fn observe_integer_returns(
    calls: &FunctionCallSites,
) -> Option<Result<Vec<IntegerReturningPath<'_>>, IntegerCheckFailure<'_>>> {
    match observe_integer_returns_in_scope(calls, None)? {
        Ok(ProofCoverage::Applicable(paths)) => Some(Ok(paths)),
        Ok(ProofCoverage::NoApplicableInputs) => None,
        Err(failure) => Some(Err(failure)),
    }
}

fn observe_integer_returns_in_scope<'a>(
    calls: &'a FunctionCallSites,
    entry_filter: Option<&CheckerPredicate>,
) -> Option<Result<ProofCoverage<Vec<IntegerReturningPath<'a>>>, IntegerCheckFailure<'a>>> {
    let tail = integer_tail_type(calls)?;
    let mut paths = Vec::new();
    match run_integer_callable(calls, Some(tail), entry_filter, Some(&mut paths), None)? {
        Ok(ProofCoverage::Applicable(())) if !paths.is_empty() => {
            Some(Ok(ProofCoverage::Applicable(paths)))
        }
        Ok(ProofCoverage::Applicable(())) => None,
        Ok(ProofCoverage::NoApplicableInputs) => Some(Ok(ProofCoverage::NoApplicableInputs)),
        Err(failure) => Some(Err(failure)),
    }
}

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

pub(super) fn check_integer_return_bound_where<'a>(
    calls: &'a FunctionCallSites,
    operator: &BinaryOperator,
    bound: &BigInt,
    entry_filter: &CheckerPredicate,
) -> Option<Result<ProofCoverage<bool>, IntegerCheckFailure<'a>>> {
    check_integer_return_bound_in_scope(calls, operator, bound, Some(entry_filter), None)
}

pub(super) fn check_integer_return_bound_with_shows<'a>(
    calls: &'a FunctionCallSites,
    operator: &BinaryOperator,
    bound: &BigInt,
    entry_filter: Option<&CheckerPredicate>,
    proof: &IntegerBranchProof<'_>,
) -> Option<Result<ProofCoverage<bool>, IntegerCheckFailure<'a>>> {
    check_integer_return_bound_in_scope(calls, operator, bound, entry_filter, Some(proof))
}

fn check_integer_return_bound_in_scope<'a>(
    calls: &'a FunctionCallSites,
    operator: &BinaryOperator,
    bound: &BigInt,
    entry_filter: Option<&CheckerPredicate>,
    proof: Option<&IntegerBranchProof<'_>>,
) -> Option<Result<ProofCoverage<bool>, IntegerCheckFailure<'a>>> {
    let relation = IntegerRelation::from_operator(operator)?;
    let paths = match observe_integer_returns_in_scope(calls, entry_filter)? {
        Ok(ProofCoverage::Applicable(paths)) => paths,
        Ok(ProofCoverage::NoApplicableInputs) => return Some(Ok(ProofCoverage::NoApplicableInputs)),
        Err(failure) => return Some(Err(failure)),
    };
    for path in paths {
        if !path.output_satisfies(relation, bound)? {
            return Some(Ok(ProofCoverage::Applicable(false)));
        }
        if let Some(proof) = proof {
            if !path.shows_branch(proof)? {
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

fn run_integer_callable<'a>(
    calls: &'a FunctionCallSites,
    tail: Option<&'a ScalarType>,
    entry_filter: Option<&CheckerPredicate>,
    returns: Option<&mut Vec<IntegerReturningPath<'a>>>,
    imported_local: Option<ImportedLocal<'a>>,
) -> Option<Result<ProofCoverage<()>, IntegerCheckFailure<'a>>> {
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
    let root = boundary
        .parameters
        .iter()
        .map(|parameter| IntegerSet::from_type(&parameter.ty).map(Rc::new))
        .collect::<Option<IntegerRow>>()?;
    let mut root = IntegerInputs(vec![root]);
    if let Some(filter) = entry_filter {
        let (applicable, _, _) = root.split(&roots, filter)?;
        if applicable.is_unreachable() {
            return Some(Ok(ProofCoverage::NoApplicableInputs));
        }
        root = applicable;
    }
    let result = walk_integer_block(calls, &mut roots, calls.structure.root, root, tail, returns)?;
    if roots.imported_local.is_some() {
        return None;
    }
    match result {
        IntegerControl::Continue(_) | IntegerControl::Returned => {
            Some(Ok(ProofCoverage::Applicable(())))
        }
        IntegerControl::Fail(failure) => Some(Err(failure)),
    }
}

fn walk_integer_block<'a>(
    calls: &'a FunctionCallSites,
    roots: &mut IntegerRoots<'a>,
    block: BlockOrdinal,
    mut path: IntegerInputs,
    tail: Option<&'a ScalarType>,
    mut returns: Option<&mut Vec<IntegerReturningPath<'a>>>,
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
                    let (passing, failing, _) = path.split(roots, directive.predicate.as_ref()?)?;
                    match directive.operation {
                        CheckerOperation::Require if !failing.is_unreachable() => {
                            return Some(IntegerControl::Fail(IntegerCheckFailure {
                                directive: expression,
                                state: failing,
                                axes: roots.axes.iter().map(|(binding, _)| *binding).collect(),
                            }));
                        }
                        CheckerOperation::Narrow if passing.is_unreachable() => {
                            return Some(IntegerControl::Fail(IntegerCheckFailure {
                                directive: expression,
                                state: failing,
                                axes: roots.axes.iter().map(|(binding, _)| *binding).collect(),
                            }));
                        }
                        CheckerOperation::Require | CheckerOperation::Narrow => path = passing,
                    }
                } else {
                    let ConditionalRegion::Unit { condition, then_block, .. } =
                        calls.conditionals.get(&expression)?
                    else {
                        return None;
                    };
                    let predicate = structure.checker_branches.get(condition)?;
                    let (yes, no, axis) = path.split(roots, predicate)?;
                    match walk_integer_block(calls, roots, *then_block, yes, None, None)? {
                        IntegerControl::Continue(then_path) => path = then_path.join(no, axis),
                        failure @ IntegerControl::Fail(_) => return Some(failure),
                        IntegerControl::Returned => return None,
                    }
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
                        match walk_integer_block(calls, roots, arm, incoming, Some(expected), returns.as_deref_mut())? {
                            IntegerControl::Returned => {}
                            failure @ IntegerControl::Fail(_) => return Some(failure),
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
                if block == structure.root
                    && roots.imported_local.as_ref().is_some_and(|local| local.position == item.position)
                {
                    let local = roots.imported_local.take()?;
                    let origin = structure.checker_integer_call_locals.get(&(block, item.position))?;
                    let [receiver] = binding.receivers() else {
                        return None;
                    };
                    let applies = match &origin.argument {
                        CheckerIntegerCallArgument::Literal(value) =>
                            local.guard.is_none_or(|(relation, bound)| relation.accepts(value.cmp(bound))),
                        CheckerIntegerCallArgument::PlainName => {
                            let PlainNameIdentity::Lexical(argument) =
                                calls.prepared_plain_name(origin.argument_ordinal)?
                            else {
                                return None;
                            };
                            let (axis, ty) = roots.lookup(argument)?;
                            if structure.checked_types.expressions.get(&origin.argument_ordinal)? != ty {
                                return None;
                            }
                            local.guard.is_none_or(|(relation, bound)| {
                                path.0.iter().all(|row| row[axis].every(relation, bound))
                            })
                        }
                    };
                    if !applies {
                        return None;
                    }
                    roots.add(&origin.binding, &receiver.ty);
                    path.append_local(local.values);
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
