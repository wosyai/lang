use super::*;

#[derive(Default)]
pub(super) struct AggregateFlowFacts {
    pub(super) allocations: BTreeMap<ReferenceBindingId, CfgPointId>,
    pub(super) allocation_slots: BTreeMap<CfgPointId, (usize, usize)>,
    pub(super) views: BTreeMap<CfgPointId, CfgPointId>,
    pub(super) plain_literals: BTreeSet<CfgPointId>,
    pub(super) carrier_literals: BTreeSet<CfgPointId>,
    pub(super) scalar_cells: BTreeSet<CfgPointId>,
    pub(super) raw_addresses: BTreeMap<CfgPointId, (ReferenceBindingId, bool)>,
    pub(super) opaque_container_inputs: BTreeMap<CfgPointId, Vec<CfgPointId>>,
    pub(super) raw_formals: BTreeMap<usize, ReferenceBindingId>,
    pub(super) raw_calls: BTreeMap<CfgPointId, (Vec<usize>, bool)>,
    pub(super) read_consumers: BTreeSet<CfgPointId>,
    pub(super) read_fields: BTreeMap<usize, ScalarStructFieldId>,
    pub(super) read_calls: BTreeMap<CfgPointId, ReadCall>,
    pub(super) raw_nulls: BTreeSet<CfgPointId>,
    pub(super) plain_aliases: BTreeMap<CfgPointId, ReferenceBindingId>,
    pub(super) literals: BTreeMap<CfgPointId, Vec<(OccurrenceOrdinal, CfgPointId)>>,
    pub(super) installations: BTreeMap<CfgPointId, (CfgPointId, BlockPosition)>,
    pub(super) calls: BTreeMap<CfgPointId, OccurrenceOrdinal>,
    pub(super) projected_reads: BTreeMap<CfgPointId, bool>,
    pub(super) scope_exits: BTreeMap<CfgPointId, BlockOrdinal>,
    pub(super) formals: BTreeMap<usize, ReferenceBindingId>,
    pub(super) input_moves: BTreeMap<CfgPointId, (OccurrenceOrdinal, BTreeMap<usize, ArgumentProvenance>)>,
    pub(super) temporary_arguments: BTreeSet<CfgPointId>,
    pub(super) formal_forward_shape: Option<(CfgPointId, CfgPointId)>,
    pub(super) array_formal: Option<ReferenceBindingId>,
    pub(super) array_calls: BTreeMap<CfgPointId, (OccurrenceOrdinal, CfgPointId, ReferenceBindingId)>,
    pub(super) array_return_move: Option<CfgPointId>,
    pub(super) array_installs: BTreeSet<CfgPointId>,
    pub(super) local_array_copy_allocation: Option<CfgPointId>,
    pub(super) local_array_nulls: BTreeSet<CfgPointId>,
    pub(super) local_array_failure_arms: BTreeMap<CfgPointId, BlockOrdinal>,
}

impl AggregateFlowFacts {
    pub(super) fn observe_binding(
        &mut self,
        cfg: &ScalarReferenceCfg,
        point: CfgPointId,
        initializer: Option<CfgPointId>,
        allocation: bool,
        ty: &ScalarType,
        position: BlockPosition,
    ) {
        let CfgPointKind::Bind { binding } = &cfg.points[point.0].kind else {
            unreachable!("aggregate ownership binding point");
        };
        match ty {
            ScalarType::RuntimeArray { element, .. } if allocation => {
                self.allocations.insert(binding.clone(), point);
                if self.allocations.len() == 1 && ordinary_value(element) {
                    self.local_array_copy_allocation = Some(point);
                } else {
                    self.local_array_copy_allocation = None;
                }
                self.allocation_slots
                    .insert(point, (position.block.0, position.item));
            }
            ScalarType::RawPointer(_) | ScalarType::Struct(_) if !allocation => {
                if let Some(initializer) = initializer {
                    self.installations.insert(point, (initializer, position));
                }
            }
            _ => {}
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum OwnerSource {
    Allocation(CfgPointId),
    Call(CfgPointId),
    Formal(usize),
}

#[derive(Clone, Copy, Debug)]
pub(super) enum ReadCall {
    Offset,
    Load,
    Direct,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum ReadSource {
    Owner(usize),
    Formal(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Value {
    ScalarCell,
    RawFormal(usize),
    ReadOffset(ReadSource),
    RawCell { point: CfgPointId, owner: Option<usize> },
    Plain(CfgPointId),
    View(OwnerSource),
    Aggregate {
        backing: OwnerSource,
        field: Option<OccurrenceOrdinal>,
    },
    Unknown,
}

impl Value {
    fn read_source(self) -> Option<ReadSource> {
        match self {
            Self::RawFormal(slot) => Some(ReadSource::Formal(slot)),
            Self::View(OwnerSource::Formal(slot)) => Some(ReadSource::Owner(slot)),
            Self::ReadOffset(source) => Some(source),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum InputOrigin {
    Owner(OwnerSource),
    Formal(usize),
    Immediate(CfgPointId),
    Plain(CfgPointId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum ArgumentProvenance {
    WholeMove(CfgPointId),
    Immediate(CfgPointId),
    Unmoved,
}

pub(super) fn same_runtime_array_type(left: &ScalarType, right: &ScalarType) -> bool {
    match (left, right) {
        (ScalarType::RuntimeArray { element: left, .. }, ScalarType::RuntimeArray { element: right, .. }) => {
            scalar_type_identity(left) == scalar_type_identity(right)
        }
        _ => false,
    }
}

pub(super) fn array_roundtrip_signature(parameters: &[ScalarType], outputs: &ScalarOutputSequence) -> bool {
    let Some((input @ ScalarType::RuntimeArray { .. }, rest)) = parameters.split_first() else {
        return false;
    };
    let Some((output, tail)) = outputs.outputs.split_first() else { return false };
    same_runtime_array_type(&output.ty, input)
        && rest.iter().all(|ty| ordinary_value(ty) || matches!(ty, ScalarType::Enum(_))
            || matches!(ty, ScalarType::CheckedReference { inner, .. } if ordinary_value(inner)))
        && tail.iter().all(|out| ordinary_value(&out.ty) || matches!(out.ty, ScalarType::Enum(_)))
}

pub(super) fn ordinary_formal_outputs(outputs: &[RootOutputType]) -> bool {
    match outputs {
        [] => true,
        [output] => output.ty == ScalarType::Unit || ordinary_value(&output.ty),
        _ => false,
    }
}

pub(super) fn checked_scalar_output(outputs: &[RootOutputType]) -> bool {
    matches!(outputs,
        [RootOutputType { output: 0, ty: ScalarType::CheckedReference { mutability: ScalarReferenceMutability::Shared, inner } }]
            if ordinary_value(inner))
}

pub(super) fn ordinary_value(ty: &ScalarType) -> bool {
    matches!(ty, ScalarType::Bool | ScalarType::Char | ScalarType::I8 | ScalarType::I16 | ScalarType::I32 |
        ScalarType::I64 | ScalarType::I128 | ScalarType::U8 | ScalarType::U16 | ScalarType::U32 |
        ScalarType::U64 | ScalarType::U128 | ScalarType::F32 | ScalarType::F64)
}

#[derive(Clone, Default, PartialEq)]
pub(super) struct AggregateEdgeState {
    values: BTreeMap<CfgPointId, Value>,
    bindings:
        BTreeMap<ReferenceBindingId, (Option<Value>, ReferenceScopeId, Option<BlockPosition>)>,
    moved: BTreeSet<OwnerSource>,
    input_moves: BTreeMap<CfgPointId, InputOrigin>,
    pending_temporaries: BTreeMap<CfgPointId, ReferenceScopeId>,
    array_formal_live: bool,
    array_formal_pending: Option<CfgPointId>,
    array_dependencies: BTreeSet<CfgPointId>,
    local_array: Option<LocalArrayLife>,
}

#[derive(Clone, Default, PartialEq)]
struct LocalArrayLife {
    live: bool,
    pending: Option<CfgPointId>,
    invalid: bool,
    has_rebind: bool,
}

impl AggregateEdgeState {
    fn forget_plain(&mut self, binding: &ReferenceBindingId) {
        if let Some((value @ Some(Value::Plain(_)), _, _)) = self.bindings.get_mut(binding) {
            *value = None;
        }
    }
}

#[derive(Default)]
pub(super) struct AggregateFlowResult {
    pub(super) array_invalid: bool,
    pub(super) array_calls: BTreeMap<CfgPointId, (OccurrenceOrdinal, CfgPointId, ReferenceBindingId)>,
    pub(super) array_returns: BTreeSet<Option<BTreeSet<CfgPointId>>>,
    pub(super) array_rebinds: BTreeMap<CfgPointId, (CfgPointId, ReferenceBindingId)>,
    pub(super) local_array_failure_arms: BTreeMap<usize, Option<CfgPointId>>,
    pub(super) allocation_slots: BTreeMap<CfgPointId, (usize, usize)>,
    pub(super) calls: BTreeMap<CfgPointId, OccurrenceOrdinal>,
    pub(super) returns: BTreeMap<CfgPointId, BTreeSet<Option<OwnerSource>>>,
    pub(super) endpoints: BTreeMap<(CfgPointId, usize), BTreeSet<Option<OwnerSource>>>,
    pub(super) unsupported: BTreeSet<CfgPointId>,
    abandoned_temporaries: BTreeMap<CfgPointId, CfgPointId>,
    crossed_temporary_calls: BTreeMap<CfgPointId, BTreeSet<CfgPointId>>,
    pub(super) returning_leaf: bool,
    pub(super) checked_result_origins: BTreeSet<ReferenceOriginId>,
    pub(super) independent_checked_result: bool,
    pub(super) formals: BTreeMap<usize, ReferenceBindingId>,
    pub(super) formal_unsupported: BTreeSet<usize>,
    pub(super) formal_forward: Option<(usize, CfgPointId, usize, usize)>,
    pub(super) checked_successor_candidate: Option<CfgPointId>,
    pub(super) checked_successor_contract: bool,
    pub(super) checked_successor_result: bool,
    raw_invalid: Option<CfgPointId>,
    pub(super) read_formals: BTreeSet<usize>,
    read_invalid: BTreeMap<ReadSource, CfgPointId>,
    read_loads: BTreeSet<ReadSource>,
    read_active: BTreeSet<ReadSource>,
    read_edges: BTreeMap<ReadSource, BTreeSet<(CfgPointId, usize)>>,
    raw_terminals: BTreeSet<(CfgPointId, Vec<usize>)>,
    raw_uses: BTreeSet<(CfgPointId, usize, Vec<usize>)>,
    pub(super) inputs: BTreeMap<(CfgPointId, usize, usize, ArgumentProvenance), BTreeSet<Option<InputOrigin>>>,

}

impl AggregateFlowResult {
    fn invalidate_raw(&mut self, point: CfgPointId) {
        self.raw_invalid = Some(self.raw_invalid.map_or(point, |previous| previous.min(point)));
    }

    fn account_raw(&mut self, point: CfgPointId, value: Option<&Value>) {
        if let Some(source) = value.and_then(|value| value.read_source()) {
            self.read_invalid.entry(source).or_insert(point);
        }
        match value {
            Some(Value::RawFormal(_) | Value::RawCell { owner: None, .. }) => {
                self.invalidate_raw(point);
            }
            Some(Value::RawCell { owner: Some(slot), .. } | Value::View(OwnerSource::Formal(slot)) |
                Value::Aggregate { backing: OwnerSource::Formal(slot), .. }) => {
                self.formal_unsupported.insert(*slot);
            }
            _ => {}
        }
    }

    pub(super) fn has_observations(&self) -> bool {
        !self.array_returns.is_empty() || !self.array_rebinds.is_empty()
            || self.returning_leaf || !self.calls.is_empty() || !self.returns.is_empty() || !self.formals.is_empty() || !self.inputs.is_empty() || !self.raw_terminals.is_empty() || !self.read_formals.is_empty()
    }
}

pub(super) fn returning_leaf_point(kind: &CfgPointKind) -> bool {
    match kind {
        CfgPointKind::Entry
        | CfgPointKind::ScopeEnter
        | CfgPointKind::ScopeExit
        | CfgPointKind::Read { .. }
        | CfgPointKind::ReturnOutput { checked: false, .. }
        | CfgPointKind::Return => true,
        CfgPointKind::Assign { target, .. } => target.projections.is_empty(),
        CfgPointKind::Evaluate(expression) => matches!(expression,
            ScalarExpression::Integer { .. } | ScalarExpression::Float { .. }
            | ScalarExpression::Boolean { .. } | ScalarExpression::Char { .. }
            | ScalarExpression::Name { .. } | ScalarExpression::Binary {
                operator: BinaryOperator::Add | BinaryOperator::Subtract | BinaryOperator::Multiply,
                ..
            }),
        _ => false,
    }
}

impl AggregateFlowFacts {
    pub(super) fn entry_state(&self) -> AggregateEdgeState {
        let mut state = AggregateEdgeState {
            array_formal_live: self.array_formal.is_some(),
            local_array: (self.local_array_copy_allocation.is_some() && !self.local_array_failure_arms.is_empty())
                .then(LocalArrayLife::default),
            ..AggregateEdgeState::default()
        };
        for (&slot, binding) in &self.formals {
            state.bindings.insert(binding.clone(), (
                Some(Value::Aggregate { backing: OwnerSource::Formal(slot), field: None }),
                ReferenceScopeId(0),
                None,
            ));
        }
        for (&slot, binding) in &self.raw_formals {
            state.bindings.insert(binding.clone(), (Some(Value::RawFormal(slot)), ReferenceScopeId(0), None));
        }
        state
    }

    pub(super) fn tracks_read_sources(&self) -> bool {
        !self.formals.is_empty() || !self.raw_formals.is_empty()
    }

    pub(super) fn observes_values(&self) -> bool {
        self.array_formal.is_some() || !self.array_calls.is_empty()
            || !self.views.is_empty() || !self.calls.is_empty() || !self.formals.is_empty() || !self.input_moves.is_empty() || !self.raw_formals.is_empty()
    }

    pub(super) fn visit(
        &self,
        point: &ReferenceFlowPoint,
        state: &mut AggregateEdgeState,
        result: &mut AggregateFlowResult,
        sites: &FunctionCallSites,
    ) {
        let id = point.id;
        if let Some(local) = &mut state.local_array {
            let (binding, &allocation) = self.allocations.first_key_value().expect("local array demand");
            if id == allocation {
                local.invalid |= local.live || local.pending.is_some() || point.scope != ReferenceScopeId(0);
                local.live = true;
            }
            if self.views.get(&id) == Some(&allocation) {
                local.invalid = true;
            }
            match &point.kind {
                CfgPointKind::Borrow { place, .. } if &place.binding == binding => local.invalid = true,
                CfgPointKind::Move { place, owner, .. } if owner == binding => {
                    if local.live && local.pending.is_none() && place.projections.is_empty() && &place.binding == binding {
                        local.live = false;
                        local.pending = Some(id);
                    } else {
                        local.invalid = true;
                    }
                }
                CfgPointKind::DeferredMove { owner, .. } if owner == binding => local.invalid = true,
                CfgPointKind::MoveThrough { .. } | CfgPointKind::DeferredMoveThrough { .. }
                | CfgPointKind::Invalidate { .. } | CfgPointKind::Release { .. } => local.invalid = true,
                CfgPointKind::Call { output: 0, .. } if local.pending.is_some() => {
                    if !self.array_calls.get(&id).is_some_and(|(_, moved, actual)| actual == binding && Some(*moved) == local.pending) {
                        local.invalid = true;
                    }
                }
                CfgPointKind::Assign { target, rhs_point, .. } if &target.binding == binding && target.projections.is_empty() => {
                    if self.array_installs.contains(&id) && self.array_calls.get(rhs_point)
                        .is_some_and(|(_, moved, actual)| actual == binding && Some(*moved) == local.pending)
                    {
                        local.pending = None;
                        local.live = true;
                        local.has_rebind = true;
                    } else {
                        local.invalid = true;
                    }
                }
                _ => {}
            }
            if let Some(&arm) = self.local_array_failure_arms.get(&id) {
                let witness = (local.live && !local.invalid && local.pending.is_none() && local.has_rebind)
                    .then_some(allocation);
                result.local_array_failure_arms
                    .entry(arm.0)
                    .and_modify(|known| {
                        if *known != witness {
                            *known = None;
                        }
                    })
                    .or_insert(witness);
            }
        }
        if let CfgPointKind::Assign { target, rhs_point, .. } = &point.kind {
            if self.array_installs.contains(&id) && target.projections.is_empty() && self.array_calls.get(rhs_point)
                .is_some_and(|(_, _, binding)| *binding == target.binding)
            {
                result.array_rebinds.insert(id, (*rhs_point, target.binding.clone()));
            }
        }
        if let Some(formal) = &self.array_formal {
            match &point.kind {
                CfgPointKind::Borrow { place, .. } if &place.binding == formal => {
                    result.array_invalid = true;
                }
                CfgPointKind::Move { place, owner, .. } if owner == formal => {
                    if state.array_formal_live && place.projections.is_empty() && &place.binding == formal {
                        state.array_formal_live = false;
                        state.array_formal_pending = Some(id);
                    } else {
                        result.array_invalid = true;
                    }
                }
                CfgPointKind::Assign { target, rhs_point, .. }
                    if &target.binding == formal && target.projections.is_empty() =>
                {
                    if self.array_installs.contains(&id) && self.array_calls.get(rhs_point)
                        .is_some_and(|(_, moved, binding)| binding == formal && Some(*moved) == state.array_formal_pending)
                    {
                        state.array_formal_pending = None;
                        state.array_formal_live = true;
                        state.array_dependencies.insert(*rhs_point);
                    } else {
                        result.array_invalid = true;
                    }
                }
                CfgPointKind::ReturnOutput { output: 0, .. } => {
                    result.array_returns.insert(
                        (state.array_formal_pending.is_some()
                            && state.array_formal_pending == self.array_return_move)
                            .then(|| state.array_dependencies.clone())
                    );
                }
                _ => {}
            }
        }
        if !state.pending_temporaries.is_empty() {
            match &point.kind {
                CfgPointKind::Call {
                    output: 0,
                    target: ResolvedCallTarget::Core(CoreOperationId::SystemPanic)
                        | ResolvedCallTarget::ExternCallable { .. }
                        | ResolvedCallTarget::LocalCallable(_)
                        | ResolvedCallTarget::CallableField { .. },
                    ..
                } | CfgPointKind::Return => {
                    for producer in state.pending_temporaries.keys() {
                        result.abandoned_temporaries.entry(*producer).or_insert(id);
                    }
                }
                CfgPointKind::ScopeExit => {
                    state.pending_temporaries.retain(|producer, scope| {
                        if *scope != point.scope { return true }
                        result.abandoned_temporaries.entry(*producer).or_insert(id);
                        false
                    });
                }
                _ => {}
            }
        }
        match &point.kind {
            CfgPointKind::ReadPlace { pointer_point: Some(pointer), .. }
            | CfgPointKind::AssignThrough { pointer_point: pointer, .. }
            | CfgPointKind::Invalidate { address_point: pointer, .. }
            | CfgPointKind::Release { target: ReleaseTarget::Checked { address_point: pointer, .. } } => {
                result.account_raw(id, state.values.get(pointer));
            }
            CfgPointKind::Release { target: ReleaseTarget::Raw { binding } } => {
                result.account_raw(id, state.bindings.get(binding).and_then(|(value, _, _)| value.as_ref()));
            }
            _ => {}
        }
        if let Some(operands) = self.opaque_container_inputs.get(&id) {
            for operand in operands {
                result.account_raw(id, state.values.get(operand));
            }
        }
        if let Some(binding) = self.plain_aliases.get(&id) {
            state.forget_plain(binding);
        }
        let value = if let Some((root, whole)) = self.raw_addresses.get(&id) {
            match state.bindings.get(root) {
                Some((Some(Value::Aggregate { backing: OwnerSource::Formal(slot), field: Some(_) }), _, Some(_))) if *whole =>
                    Some(Value::RawCell { point: id, owner: Some(*slot) }),
                Some((Some(Value::ScalarCell), _, Some(_))) if *whole =>
                    Some(Value::RawCell { point: id, owner: None }),
                binding => {
                    result.account_raw(id, binding.and_then(|(value, _, _)| value.as_ref()));
                    self.views.get(&id).map(|&base| Value::View(OwnerSource::Allocation(base)))
                }

            }
        } else if self.scalar_cells.contains(&id) {
            Some(Value::ScalarCell)
        } else if let Some(&base) = self.views.get(&id) {
            Some(Value::View(OwnerSource::Allocation(base)))
        } else if self.plain_literals.contains(&id) {
            Some(Value::Plain(id))
        } else if let Some(inputs) = self.literals.get(&id) {
            let mut member = None;
            for &(field, value) in inputs {
                let next = match state.values.get(&value) {
                    Some(Value::View(backing)) => Value::Aggregate { backing: *backing, field: Some(field) },
                    Some(value @ (Value::Aggregate { .. } | Value::RawFormal(_) | Value::ReadOffset(_) | Value::RawCell { .. })) => {
                        result.account_raw(id, Some(value));
                        Value::Unknown
                    }
                    Some(Value::Unknown) => Value::Unknown,
                    Some(Value::Plain(_) | Value::ScalarCell) | None => continue,
                };
                member = Some(if let Some(previous) = member {
                    result.account_raw(id, Some(&previous));
                    result.account_raw(id, Some(&next));
                    Value::Unknown
                } else {
                    next
                });
            }
            member
        } else {
            match &point.kind {
                CfgPointKind::Borrow { place, .. } => {
                    state.forget_plain(&place.binding);
                    result.account_raw(id, state.bindings.get(&place.binding).and_then(|(value, _, _)| value.as_ref()));
                    None
                }
                CfgPointKind::Read { binding } => {
                    state.bindings.get(binding).and_then(|(value, _, _)| *value)
                }
                CfgPointKind::ReadPlace { candidates, .. }
                    if self.projected_reads.contains_key(&id) =>
                {
                    let mut chosen = None;
                    for place in candidates {
                        let Some(value) = state.bindings.get(&place.binding).and_then(|(value, _, _)| value.as_ref()) else {
                            continue;
                        };
                        let projected = match (value, place.projections.as_slice()) {
                            (
                                Value::Aggregate {
                                    backing,
                                    field: Some(field),
                                },
                                [ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(
                                    actual,
                                ))],
                            ) if sites.structure.literal_fields.get(field) == Some(actual) => {
                                Some(Value::View(*backing))
                            }
                            (
                                Value::Aggregate { backing: OwnerSource::Formal(_), .. },
                                [ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(_))],
                            ) if self.projected_reads.get(&id) == Some(&true) => {
                                let Value::Aggregate { backing, .. } = value else { unreachable!() };
                                Some(Value::View(*backing))
                            }
                            (Value::Aggregate { .. }, []) => Some(*value),
                            (_, []) => Some(*value),
                            (Value::Plain(_), _) => None,
                            _ => {
                                result.account_raw(id, Some(value));
                                Some(Value::Unknown)
                            }
                        };
                        if let Some(projected) = projected {
                            if let Value::View(OwnerSource::Formal(owner)) = projected {
                                if !matches!(place.projections.as_slice(),
                                    [ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(field))]
                                        if self.read_fields.get(&owner) == Some(field))
                                {
                                    result.read_invalid.entry(ReadSource::Owner(owner)).or_insert(id);
                                }
                            }
                            chosen = Some(if let Some(previous) = chosen {
                                result.account_raw(id, Some(&previous));
                                result.account_raw(id, Some(&projected));
                                Value::Unknown
                            } else {
                                projected
                            });
                        }
                    }
                    chosen
                }
                CfgPointKind::Bind { binding } => {
                    if let Some(&(rhs, position)) = self.installations.get(&id) {
                        match state.values.get(&rhs) {
                            Some(Value::Aggregate { backing: OwnerSource::Formal(_), field: Some(_) }) if self.carrier_literals.contains(&rhs) => {}
                            value => result.account_raw(id, value),
                        }
                        state.bindings.insert(
                            binding.clone(),
                            (state.values.get(&rhs).copied().filter(|value| !matches!(value, Value::Plain(origin) if *origin != rhs)), point.scope, Some(position)),
                        );
                    }
                    None
                }
                CfgPointKind::Assign {
                    target, rhs_point, ..
                } => {
                    for value in state.bindings.get(&target.binding).and_then(|(value, _, _)| value.as_ref()).into_iter().chain(state.values.get(rhs_point)) {
                        result.account_raw(id, Some(value));
                    }
                    if target.projections.is_empty() {
                        let source = state.values.get(rhs_point).copied().filter(|value| !matches!(value, Value::Plain(_)));
                        if let Some((previous, _, _)) = state.bindings.get(&target.binding) {
                            if matches!(previous, Some(Value::Aggregate { .. })) {
                                result.unsupported.insert(id);
                            }
                        }
                        if let Some(source) = source {
                            if matches!(source, Value::Aggregate { .. }) {
                                result.unsupported.insert(id);
                            }
                            let (scope, position) = state
                                .bindings
                                .get(&target.binding)
                                .map(|(_, scope, position)| (*scope, *position))
                                .unwrap_or((point.scope, None));
                            state
                                .bindings
                                .insert(target.binding.clone(), (Some(source), scope, position));
                        } else if let Some((value, _, _)) = state.bindings.get_mut(&target.binding)
                        {
                            *value = None;
                        }
                    } else if let Some((value @ Some(Value::Plain(_)), _, _)) = state.bindings.get_mut(&target.binding) {
                        *value = None;
                    } else if state
                        .bindings
                        .get(&target.binding)
                        .is_some_and(|(v, _, _)| v.is_some())
                        || state.values.contains_key(rhs_point)
                    {
                        result.unsupported.insert(id);
                    }
                    None
                }
                CfgPointKind::Move { place, owner, .. } => {
                    if state
                        .bindings
                        .get(owner)
                        .is_some_and(|(value, _, _)| matches!(value, Some(Value::Aggregate { .. } | Value::Plain(_))))
                    {
                        if place.projections.is_empty() {
                            match state.bindings.remove(owner).and_then(|(value, _, _)| value) {
                                Some(Value::Aggregate { backing: OwnerSource::Formal(slot), field: None })
                                    if self.formal_forward_shape.is_some_and(|(moved, _)| moved == id)
                                        && self.formals.get(&slot) == Some(owner) => {
                                    state.input_moves.insert(id, InputOrigin::Formal(slot));
                                }
                                Some(Value::Aggregate { backing, .. }) => {
                                    state.input_moves.insert(id, InputOrigin::Owner(backing));
                                    if let OwnerSource::Formal(slot) = backing {
                                        result.formal_unsupported.insert(slot);
                                    }
                                }
                                Some(Value::Plain(origin)) => { state.input_moves.insert(id, InputOrigin::Plain(origin)); }
                                _ => {}
                            }
                        } else {
                            result.unsupported.insert(id);
                        }
                    }
                    None
                }
                CfgPointKind::AssignThrough { rhs_point, .. } => {
                    let value = state.values.get(rhs_point);
                    result.account_raw(id, value);
                    if value.is_some() {
                        result.unsupported.insert(id);
                    }
                    None
                }
                CfgPointKind::Call {
                    output: 0,
                    target,
                    argument_points,
                    ..
                } => {
                    let mut allowed_raw = None;
                    if let Some((slots, declared_unsafe)) = self.raw_calls.get(&id) {
                        if slots.len() == 2 {
                            let raw = [state.values.get(&argument_points[slots[0]]), state.values.get(&argument_points[slots[1]])];
                            match (target, raw) {
                                (ResolvedCallTarget::ExternCallable { .. }, [Some(Value::RawFormal(left)), Some(Value::RawFormal(right))])
                                    if *declared_unsafe && left != right => {
                                        result.raw_terminals.insert((id, vec![*left, *right]));
                                        allowed_raw = Some(slots);
                                    }
                                (ResolvedCallTarget::ModuleCallable { .. }, [Some(Value::RawCell { point: a, owner: left }), Some(Value::RawCell { point: b, owner: right })])
                                    if left.is_some() != right.is_some()
                                        && self.raw_addresses.get(a).map(|(root, _)| root) != self.raw_addresses.get(b).map(|(root, _)| root)
                                        && [(*a, *left), (*b, *right)].into_iter().all(|(point, owner)| {
                                            self.raw_addresses.get(&point).and_then(|(root, _)| state.bindings.get(root))
                                                .is_some_and(|(value, _, position)| position.is_some() && match (value, owner) {
                                                    (Some(Value::Aggregate { backing: OwnerSource::Formal(actual), field: Some(_) }), Some(owner)) => *actual == owner,
                                                    (Some(Value::ScalarCell), None) => true,
                                                    _ => false,
                                                })
                                        }) => {
                                        result.raw_uses.insert((id, left.or(*right).expect("one owning carrier"), slots.clone()));
                                        allowed_raw = Some(slots);
                                    }
                                _ => {}
                            }
                        }
                    }
                    let moves = self.input_moves.get(&id);
                    let ordinary_read = self.read_calls.get(&id);
                    let mut offset = None;
                    for (slot, argument) in argument_points.iter().enumerate() {
                        let value = state.values.get(argument);
                        if ordinary_read.is_some() && matches!(value, Some(Value::RawFormal(_))) {
                            result.invalidate_raw(id);
                        }
                        if let Some(source) = value.and_then(|value| value.read_source()) {
                            let live = match source {
                                ReadSource::Formal(_) => true,
                                ReadSource::Owner(owner) => !state.moved.contains(&OwnerSource::Formal(owner))
                                    && self.formals.get(&owner).and_then(|binding| state.bindings.get(binding))
                                        .is_some_and(|(value, _, _)| matches!(value, Some(Value::Aggregate {
                                            backing: OwnerSource::Formal(actual), field: None
                                        }) if *actual == owner)),
                            };
                            if live {
                                match ordinary_read {
                                    Some(ReadCall::Offset) if slot == 0 => {
                                        result.read_active.insert(source);
                                        offset = Some(Value::ReadOffset(source));
                                        continue;
                                    }
                                    Some(ReadCall::Load) if slot == 0 => {
                                        result.read_loads.insert(source);
                                        continue;
                                    }
                                    Some(ReadCall::Direct) if self.raw_calls.get(&id)
                                        .is_some_and(|(slots, _)| slots.contains(&slot)) => {
                                        result.read_active.insert(source);
                                        result.read_edges.entry(source).or_default().insert((id, slot));
                                        continue;
                                    }
                                    _ => {}
                                }
                            }
                            result.read_invalid.entry(source).or_insert(id);
                        }
                        match value {
                            Some(Value::RawFormal(_)) if allowed_raw.is_some_and(|allowed| allowed.contains(&slot)) => continue,
                            Some(Value::RawCell { .. }) if allowed_raw.is_some_and(|allowed| allowed.contains(&slot)) => continue,
                            Some(value @ (Value::RawFormal(_) | Value::ReadOffset(_) | Value::RawCell { .. })) => result.account_raw(id, Some(value)),
                            _ => {}
                        }
                        let backing = match value {
                            Some(Value::Aggregate { backing, .. } | Value::View(backing)) => Some(*backing),
                            _ => None,
                        };
                        if let Some((ordinal, actual_moves)) = moves {
                            if let Some(&moved) = actual_moves.get(&slot) {
                                let actual = match value {
                                    Some(Value::Aggregate { backing, .. }) => Some(InputOrigin::Owner(*backing)),
                                    Some(Value::Plain(origin)) => Some(InputOrigin::Plain(*origin)),
                                    _ => None,
                                };
                                let source = match moved {
                                    ArgumentProvenance::WholeMove(moved) => state.input_moves.remove(&moved).filter(|owner| {
                                        if let InputOrigin::Formal(formal) = owner {
                                            self.formal_forward_shape == Some((moved, id))
                                                && actual == Some(InputOrigin::Owner(OwnerSource::Formal(*formal)))
                                        } else {
                                            Some(*owner) == actual
                                        }
                                    }),
                                    ArgumentProvenance::Immediate(producer) if *argument == producer
                                        && actual == Some(InputOrigin::Owner(OwnerSource::Call(producer)))
                                        && matches!(target, ResolvedCallTarget::ModuleCallable { .. })
                                        && state.pending_temporaries.remove(&producer).is_some() => {
                                        Some(InputOrigin::Immediate(producer))
                                    }
                                    ArgumentProvenance::Immediate(_) => None,
                                    ArgumentProvenance::Unmoved => actual.filter(|origin| matches!(origin, InputOrigin::Plain(point) if *point == *argument)),
                                };
                                result.inputs.entry((id, ordinal.0, slot, moved)).or_default().insert(source);
                                if let Some(InputOrigin::Formal(formal)) = source {
                                    let transfer = (formal, id, ordinal.0, slot);
                                    if result.formal_forward.is_some_and(|old| old != transfer) {
                                        result.formal_unsupported.insert(formal);
                                    }
                                    result.formal_forward = Some(transfer);
                                    continue;
                                }
                                if matches!(source, Some(InputOrigin::Owner(OwnerSource::Call(_)) | InputOrigin::Immediate(_) | InputOrigin::Plain(_))) && matches!(target, ResolvedCallTarget::ModuleCallable { .. }) {
                                    continue;
                                }
                            }
                        }
                        if !matches!(target, ResolvedCallTarget::Core(CoreOperationId::Load)) {
                            match backing {
                                Some(OwnerSource::Formal(formal)) => { result.formal_unsupported.insert(formal); }
                                _ if value.is_some_and(|value| !matches!(value, Value::Plain(_) | Value::ScalarCell | Value::RawFormal(_) | Value::RawCell { .. })) => { result.unsupported.insert(id); }
                                _ => {}
                            }
                        }
                    }
                    if matches!(target, ResolvedCallTarget::ModuleCallable { .. }) {
                        for producer in state.pending_temporaries.keys() {
                            result.crossed_temporary_calls.entry(*producer).or_default().insert(id);
                        }
                    }
                    if self.temporary_arguments.contains(&id) {
                        state.pending_temporaries.insert(id, point.scope);
                    }
                    offset.or_else(|| self.calls.contains_key(&id).then_some(Value::Aggregate {
                        backing: OwnerSource::Call(id),
                        field: None,
                    }))
                }
                CfgPointKind::ConditionalOutput {
                    then_point,
                    else_point,
                } => match (state.values.get(then_point), state.values.get(else_point)) {
                    (Some(value), None) | (None, Some(value)) => {
                        result.account_raw(id, Some(value));
                        Some(*value)
                    }
                    (None, None) => None,
                    (Some(left), Some(right)) => {
                        result.account_raw(id, Some(left));
                        result.account_raw(id, Some(right));
                        Some(Value::Unknown)
                    }
                },
                CfgPointKind::ReturnOutput { value_point, .. } => {
                    let return_value = state.values.get(value_point);
                    result.account_raw(id, return_value);
                    let returning = match return_value {
                        Some(Value::Aggregate { backing, .. }) => {
                            if let OwnerSource::Formal(slot) = backing {
                                result.formal_unsupported.insert(*slot);
                            }
                            state.moved.insert(*backing);
                            Some(*backing)
                        }
                        Some(Value::View(_) | Value::Unknown) => {
                            result.unsupported.insert(id);
                            None
                        }
                        Some(Value::RawFormal(_) | Value::ReadOffset(_) | Value::RawCell { .. }) => None,
                        Some(Value::Plain(_) | Value::ScalarCell) | None => None,
                    };
                    result.returns.entry(id).or_default().insert(returning);
                    None
                }
                CfgPointKind::ScopeExit => {
                    state.bindings.retain(|_, (value, owner_scope, position)| {
                        if *owner_scope != point.scope {
                            return true;
                        }
                        if let (Some(Value::Aggregate { backing, .. }), Some(exit)) =
                            (value, self.scope_exits.get(&id))
                        {
                            if position.is_some() || matches!(backing, OwnerSource::Formal(_)) {
                                result.endpoints.entry((id, exit.0)).or_default()
                                    .insert((!state.moved.contains(backing)).then_some(*backing));
                            }
                        }
                        false
                    });
                    None
                }
                _ => None,
            }
        };
        if let Some(value) = value {
            if let Some(source) = value.read_source() {
                if !self.read_consumers.contains(&id) {
                    result.read_invalid.entry(source).or_insert(id);
                }
            }
            state.values.insert(id, value);
        } else {
            state.values.remove(&id);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClosedReturn {
    Allocation { block: usize, item: usize },
    Forward { call: OccurrenceOrdinal },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClosedCall {
    Receive,
    Forward,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClosedInput {
    Transfer(usize),
    ForwardFormal(usize),
    Plain,
}

#[derive(Debug, Default)]
pub(crate) struct ClosedBody {
    pub(crate) returning: Option<ClosedReturn>,
    pub(crate) calls: BTreeMap<usize, ClosedCall>,
    pub(crate) scope_exits: BTreeMap<usize, CfgPointId>,
    pub(crate) releases: BTreeMap<CfgPointId, Vec<usize>>,
    pub(crate) input: Option<usize>,
    pub(crate) input_mixed: bool,
    pub(crate) same_array_rebinds: BTreeSet<usize>,
    pub(crate) local_array_failure: Option<((usize, usize), usize)>,
    pub(crate) input_release: Option<CfgPointId>,
    pub(crate) input_calls: BTreeMap<usize, (usize, ClosedInput)>,
}

pub(crate) type BodyKey = (usize, usize, Option<usize>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClosedInitializerInput {
    Consume(usize),
    Immediate(usize),
    Plain,
}

#[derive(Debug, Default)]
pub(crate) struct ClosedInitializer {
    pub(crate) installs: HashMap<ByteSpan, usize>,
    pub(crate) immediate: HashMap<usize, usize>,
    pub(crate) inputs: HashMap<usize, ClosedInitializerInput>,
}

#[derive(Debug, Default)]
pub(crate) struct ClosedAggregates {
    bodies: BTreeMap<BodyKey, ClosedBody>,
    initializers: HashMap<SourceIdentity, ClosedInitializer>,
}

impl ClosedAggregates {
    pub(crate) fn initializer(&self, source: &SourceIdentity) -> Option<&ClosedInitializer> {
        self.initializers.get(source)
    }

    pub(crate) fn body(
        &self,
        module: usize,
        item: usize,
        arm: Option<usize>,
    ) -> Option<&ClosedBody> {
        self.bodies.get(&(module, item, arm))
    }
}

#[derive(Default)]
pub(crate) struct AggregateReservations {
    pub(crate) bodies: BTreeMap<BodyKey, bool>,
    pub(crate) modules: Vec<usize>,
}

#[derive(Default)]
pub(in crate::scalar) struct InstalledAggregateFacts(BTreeMap<(usize, Option<usize>), AggregateFlowResult>);

impl InstalledAggregateFacts {
    pub(super) fn insert(&mut self, key: (usize, Option<usize>), facts: AggregateFlowResult) {
        self.0.insert(key, facts);
    }
}

struct Body<'a> {
    facts: Option<&'a AggregateFlowResult>,
    cfg: &'a ScalarReferenceCfg,
    sites: &'a FunctionCallSites,
    source: &'a SourceIdentity,
    is_monomorphic: bool,
    supports_input_result: bool,
}

fn target_body(
    bodies: &HashMap<(&SourceIdentity, ByteSpan), (usize, usize)>,
    cfg: &ScalarReferenceCfg,
    point: CfgPointId,
) -> Option<BodyKey> {
    let CfgPointKind::Call {
        output: 0,
        target: ResolvedCallTarget::ModuleCallable { source, declaration_span, concrete },
        ..
    } = &cfg.points.get(point.0)?.kind else {
        return None;
    };
    let &(module, item) = bodies.get(&(source, *declaration_span))?;
    Some((module, item, match concrete {
        ConcreteCallSelection::Declaration => None,
        ConcreteCallSelection::Overload(selection) => Some(selection.arm_index),
    }))
}

fn prove_read_views(
    root: (BodyKey, ReadSource),
    bodies: &BTreeMap<BodyKey, Body<'_>>,
    declarations: &HashMap<(&SourceIdentity, ByteSpan), (usize, usize)>,
    proven: &mut BTreeSet<(BodyKey, ReadSource)>,
) -> Result<(), String> {
    if proven.contains(&root) {
        return Ok(());
    }
    let mut pending = vec![root];
    let mut seen = BTreeSet::new();
    let mut previous = BTreeMap::<_, Vec<_>>::new();
    let mut loads = Vec::new();
    while let Some(node @ (key, source)) = pending.pop() {
        if proven.contains(&node) {
            loads.push(node);
            continue;
        }
        if !seen.insert(node) {
            continue;
        }
        let body = bodies.get(&key).ok_or_else(|| format!("missing ordinary raw read body {key:?}"))?;
        let facts = body.facts.ok_or_else(|| format!("missing ordinary raw read facts {key:?}"))?;
        if matches!(source, ReadSource::Formal(slot) if !facts.read_formals.contains(&slot)
            || !body.sites.boundary.root_outputs.iter().all(|output| ordinary_value(&output.ty) || output.ty == ScalarType::Unit))
            || !body.is_monomorphic
        {
            return Err(format!("unproved ordinary raw read formal {source:?} at {key:?} in {:?}", body.source));
        }
        if let Some(point) = facts.read_invalid.get(&source) {
            return Err(format!("unproved ordinary raw read use at {point:?} for {source:?} in {:?}", body.source));
        }
        if facts.read_loads.contains(&source) {
            loads.push(node);
        }
        if let Some(edges) = facts.read_edges.get(&source) {
            for &(point, slot) in edges {
                let target = target_body(declarations, body.cfg, point)
                    .ok_or_else(|| format!("unbound ordinary raw read call at {point:?} in {:?}", body.source))?;
                let next = (target, ReadSource::Formal(slot));
                previous.entry(next).or_default().push(node);
                pending.push(next);
            }
        }
    }
    let mut closed = BTreeSet::new();
    while let Some(node) = loads.pop() {
        if closed.insert(node) {
            if let Some(edges) = previous.remove(&node) {
                loads.extend(edges);
            }
        }
    }
    if let Some(&(key, source)) = seen.difference(&closed).next() {
        return Err(format!("ordinary raw read view has no proven load for {source:?} in {:?}", bodies[&key].source));
    }
    proven.extend(seen);
    Ok(())
}

struct ModuleInput<'a> {
    source: &'a SourceIdentity,
    items: &'a [ScalarItem],
    calls: &'a ModuleCallSites,
}

#[derive(Clone, Copy)]
enum InitializerInput<'a> {
    Owned(&'a ModuleStorageInstall, BodyKey),
    Immediate(OccurrenceOrdinal, BodyKey),
    Plain,
    Unproved,
}

struct InitializerUser<'a> {
    region: &'a ModuleInitializationRegion,
    call: OccurrenceOrdinal,
    target: BodyKey,
    input: InitializerInput<'a>,
}

fn initializer_target(
    declarations: &HashMap<(&SourceIdentity, ByteSpan), (usize, usize)>,
    target: &ModuleInitializationCallTarget,
) -> Option<BodyKey> {
    let ModuleInitializationCallTarget::Direct { declaration, concrete } = target else {
        return None;
    };
    let &(module, item) = declarations.get(&(&declaration.source, declaration.range))?;
    let arm = match concrete {
        ConcreteCallSelection::Declaration => None,
        ConcreteCallSelection::Overload(selection) => Some(selection.arm_index),
    };
    Some((module, item, arm))
}

fn closed_aggregates<'a>(
    reservations: &AggregateReservations,
    module_at: impl Fn(usize) -> Option<ModuleInput<'a>>,
    result_at: impl Fn(BodyKey, &'a ScalarFunction) -> Option<(Option<&'a AggregateFlowResult>, &'a ScalarReferenceCfg)>,
    initializers: impl Iterator<Item = &'a ModuleInitializationRegion>,
) -> Result<ClosedAggregates, String> {
    let mut bodies = BTreeMap::new();
    let mut declarations = HashMap::new();
    let mut storage_uses = HashMap::<&SourceSpan, usize>::new();
    for (&key @ (module, item, arm), &is_monomorphic) in &reservations.bodies {
        let input = module_at(module).ok_or_else(|| format!("missing emitted aggregate module {module}"))?;
        let Some(ScalarItem::Function(function)) = input.items.get(item) else {
            return Err(format!("missing emitted aggregate declaration {key:?}"));
        };
        let body = arm.map_or(Some(function), |i| function.overload_arms.get(i))
            .ok_or_else(|| format!("missing emitted aggregate callable instance {key:?}"))?;
        let sites = input.calls.function_body(item, arm)
            .ok_or_else(|| format!("missing emitted aggregate call scope {key:?}"))?;
        let (facts, cfg) = result_at(key, body)
            .ok_or_else(|| format!("missing emitted aggregate result {key:?}"))?;
        declarations.insert((input.source, function.name_span), (module, item));
        for (storage, uses) in &sites.structure.module_storage_uses {
            *storage_uses.entry(storage).or_default() += uses.len();
        }
        if facts.is_some_and(AggregateFlowResult::has_observations)
            || !sites.callable_escapes.is_empty() {
            let supports_input_result = matches!(&body.signature, ScalarType::Callable { outputs, .. }
                if match outputs.outputs.as_slice() {
                    [] => true,
                    [output] => output.ty == ScalarType::Unit || ordinary_value(&output.ty)
                        || (checked_scalar_output(&sites.boundary.root_outputs)
                            && facts.is_some_and(|facts| facts.independent_checked_result || facts.checked_successor_result)),
                    [left, right] => ordinary_value(&left.ty) && ordinary_value(&right.ty),
                    _ => false,
                });
            bodies.insert(key, Body { facts, cfg, sites, source: input.source, is_monomorphic, supports_input_result });
        }
    }
    let mut outgoing = BTreeSet::new();
    let mut pending = Vec::new();
    let mut callers = BTreeMap::<BodyKey, Vec<BodyKey>>::new();
    let mut forward_edges = BTreeMap::new();
    let mut modes = BTreeMap::<(BodyKey, usize), (bool, bool)>::new();
    let mut seed_targets = BTreeMap::<BodyKey, BTreeSet<(BodyKey, usize)>>::new();
    let mut input_users = BTreeMap::<(BodyKey, usize), Vec<(BodyKey, usize, ArgumentProvenance, &BTreeSet<Option<InputOrigin>>) >>::new();
    let mut array_callsites = Vec::new();
    let mut array_leaves = BTreeSet::new();
    let mut array_neighbors = BTreeMap::<BodyKey, BTreeSet<BodyKey>>::new();
    for (&key, body) in &bodies {
        if let Some(facts) = body.facts {
            if body.is_monomorphic && !facts.array_rebinds.is_empty() {
                array_callsites.push((key, body.cfg, facts));
            }
            if body.is_monomorphic && !facts.array_invalid && !facts.array_returns.is_empty() {
                let mut grounded = false;
                let mut dependencies = BTreeSet::new();
                let valid = facts.array_returns.iter().all(|path| {
                    let Some(path) = path else { return false };
                    let mut self_call = false;
                    for &point in path {
                        let Some(target) = target_body(&declarations, body.cfg, point) else { return false };
                        self_call |= target == key;
                        dependencies.insert(target);
                    }
                    grounded |= !self_call;
                    true
                });
                if valid && grounded {
                    if dependencies.is_empty() {
                        array_leaves.insert(key);
                    } else {
                        dependencies.remove(&key);
                        array_neighbors.insert(key, dependencies);
                    }
                }
            }
            for (&(point, ordinal, slot, moved), origins) in &facts.inputs {
                if let Some(target) = target_body(&declarations, body.cfg, point) {
                    let receiver = (target, slot);
                    input_users.entry(receiver).or_default().push((key, ordinal, moved, origins));
                    let mode = modes.entry(receiver).or_default();
                    for source in origins {
                        match *source {
                            Some(InputOrigin::Owner(OwnerSource::Call(producer)) | InputOrigin::Immediate(producer)) => {
                                if let Some(origin) = target_body(&declarations, body.cfg, producer) {
                                    seed_targets.entry(origin).or_default().insert(receiver);
                                }
                            }
                            Some(InputOrigin::Plain(_)) => mode.1 = true,
                            _ => {}
                        }
                    }
                    if let Some((formal, call, forward_ordinal, forward_slot)) = facts.formal_forward {
                        if (point, ordinal, slot) == (call, forward_ordinal, forward_slot) {
                            forward_edges.insert((key, formal), (target, slot));
                        }
                    }
                }
            }
        }
        for source in body.facts.into_iter().flat_map(|facts| facts.returns.values()).flatten().flatten() {
            match *source {
                OwnerSource::Formal(_) => {}
                OwnerSource::Allocation(_) => {
                    if outgoing.insert(key) {
                        pending.push(key);
                    }
                }
                OwnerSource::Call(point) => {
                    if let Some(target) = target_body(&declarations, body.cfg, point) {
                        callers.entry(target).or_default().push(key);
                    }
                }
            }
        }
    }
    while let Some(target) = pending.pop() {
        if let Some(body_callers) = callers.remove(&target) {
            for caller in body_callers {
                if outgoing.insert(caller) {
                    pending.push(caller);
                }
            }
        }
    }
    let mut result = ClosedAggregates::default();
    for (key, cfg, facts) in array_callsites {
        for &(call, _) in facts.array_rebinds.values() {
            let Some((ordinal, _, _)) = facts.array_calls.get(&call) else { continue };
            let Some(callee) = target_body(&declarations, cfg, call) else { continue };
            if array_leaves.contains(&callee)
                || array_neighbors.get(&callee).is_some_and(|deps| deps.is_subset(&array_leaves))
            {
                result.bodies.entry(key).or_default().same_array_rebinds.insert(ordinal.0);
            }
        }
        if facts.local_array_failure_arms.is_empty() {
            continue;
        }
        let Some(plan) = result.bodies.get_mut(&key) else { continue };
        let mut failed = facts.local_array_failure_arms.iter().filter_map(|(arm, site)| {
            site.map(|site| (arm, site))
        });
        let Some((arm, allocation)) = failed.next() else { continue };
        if failed.next().is_some() {
            continue;
        }
        let CfgPointKind::Bind { binding } = &cfg.points[allocation.0].kind else { continue };
        let mut matching = facts.array_rebinds.values().filter(|(_, target)| target == binding);
        if !matching.all(|(point, _)| {
            facts.array_calls.get(point).is_some_and(|(ordinal, _, _)| plan.same_array_rebinds.contains(&ordinal.0))
        }) {
            continue;
        }
        let Some(&position) = facts.allocation_slots.get(&allocation) else { continue };
        plan.local_array_failure = Some((position, *arm));
    }
    if outgoing.is_empty() {
        return Ok(result);
    }
    let mut initializer_uses = Vec::new();
    let mut initializer_escapes = Vec::new();
    for region in initializers {
        let ModuleInitializationDetail::Occurrences(topology) = &region.detail else {
            return Err(format!("cannot close aggregate ownership across initializer in {:?}", region.source));
        };
        initializer_escapes.push((region, &topology.callable_escapes));
        for (storage, uses) in &topology.structure.module_storage_uses {
            *storage_uses.entry(storage).or_default() += uses.len();
        }
        let mut slots = HashMap::new();
        for install in topology.structure.module_storage_installs.values() {
            if let Some(consumer) = install.consumer {
                if slots.insert(consumer, install).is_some() {
                    return Err(format!("ambiguous aggregate initializing consumption in {:?}", region.source));
                }
            }
        }
        for &call in &topology.calls {
            let target = region.call_target(call)
                .ok_or_else(|| topology.missing_call_target(&region.source, call))?;
            let Some(body) = initializer_target(&declarations, target) else { continue };
            let formal = topology.checked_call_signatures.get(&call).and_then(|row| match row.parameters.as_slice() {
                [ty @ ScalarType::Struct(_)] => Some(ty),
                _ => None,
            });
            let argument = match topology.structure.edges.get(&(OccurrenceParent::Expression(call), OccurrenceEdge::CallArgument(0))) {
                Some(OccurrenceNode::Expression(ordinal)) => Some(*ordinal),
                _ => None,
            };
            let input = match (formal, argument) {
                (Some(ty), Some(argument)) if topology.structure.checked_types.expressions.get(&argument) == Some(ty) => {
                    match slots.get(&call) {
                        Some(install) => topology.call_targets.get(&install.producer)
                            .and_then(|producer| initializer_target(&declarations, producer))
                            .filter(|producer| outgoing.contains(producer))
                            .map_or(InitializerInput::Unproved, |producer| InitializerInput::Owned(install, producer)),
                        None if topology.structure.initializer_static_utf8.contains(&argument) => InitializerInput::Plain,
                        None => topology.structure.initializer_immediate.get(&call)
                            .filter(|&&producer| producer == argument)
                            .and_then(|&producer| {
                                let signature = topology.checked_call_signatures.get(&producer)?;
                                let [output] = signature.outputs.outputs.as_slice() else { return None };
                                if output.ty != *ty { return None }
                                let body = initializer_target(&declarations, topology.call_targets.get(&producer)?)?;
                                outgoing.contains(&body).then_some(InitializerInput::Immediate(producer, body))
                            }).unwrap_or(InitializerInput::Unproved),
                    }
                }
                _ => InitializerInput::Unproved,
            };
            initializer_uses.push(InitializerUser {
                region,
                call,
                target: body,
                input,
            });
        }
    }
    let mut initializer_users = BTreeMap::<(BodyKey, usize), Vec<_>>::new();
    let mut initializer_receives = HashMap::new();
    let mut initializer_owned_formals = BTreeSet::new();
    for (index, user) in initializer_uses.iter_mut().enumerate() {
        let receiver = (user.target, 0);
        let origin = match user.input {
            InitializerInput::Owned(install, producer)
                if storage_uses.get(&install.storage) == Some(&1) => Some((install.producer, producer)),
            InitializerInput::Immediate(call, producer) => Some((call, producer)),
            InitializerInput::Owned(..) => {
                user.input = InitializerInput::Unproved;
                None
            }
            InitializerInput::Plain => {
                modes.entry(receiver).or_default().1 = true;
                None
            }
            InitializerInput::Unproved => None,
        };
        if let Some((call, producer)) = origin {
            if initializer_receives.insert((&user.region.source, call), receiver).is_some() {
                return Err(format!("ambiguous aggregate initializing receipt in {:?}", user.region.source));
            }
            initializer_owned_formals.insert(receiver);
            seed_targets.entry(producer).or_default().insert(receiver);
            modes.entry(receiver).or_default();
            input_users.entry(receiver).or_default();
        }
        initializer_users.entry(receiver).or_default().push(index);
    }
    let mut input_plans = BTreeMap::<BodyKey, (usize, Option<(CfgPointId, usize)>, bool)>::new();
    let mut input_calls = BTreeMap::<BodyKey, BTreeMap<usize, (usize, ClosedInput)>>::new();
    let mut discharged_by_inputs = BTreeSet::<(BodyKey, CfgPointId)>::new();
    let active = |caller, source| {
        let producer = match source {
            Some(InputOrigin::Owner(OwnerSource::Call(producer)) | InputOrigin::Immediate(producer)) => producer,
            _ => return None,
        };
        target_body(&declarations, bodies[&caller].cfg, producer)
            .is_some_and(|target| outgoing.contains(&target)).then_some(producer)
    };
    let mut demand = Vec::new();
    for producer in &outgoing {
        if let Some(targets) = seed_targets.remove(producer) {
            for target in targets {
                let mode = modes.get_mut(&target).expect("indexed owning formal mode");
                if !mode.0 {
                    mode.0 = true;
                    demand.push(target);
                }
            }
        }
    }
    while let Some(formal) = demand.pop() {
        let Some(next) = forward_edges.get(&formal) else { continue };
        let source = modes[&formal];
        let dest = modes.get_mut(next).ok_or_else(|| "missing forwarded formal input use".to_owned())?;
        let combined = (dest.0 || source.0, dest.1 || source.1);
        if *dest != combined {
            *dest = combined;
            demand.push(*next);
        }
    }
    let mut forward_predecessors = BTreeMap::<_, Vec<_>>::new();
    let mut terminal_formals = Vec::new();
    let mut proven_read_views = BTreeSet::new();
    for ((callee, slot), users) in input_users {
        let (owned, mixed) = modes[&(callee, slot)];
        if !owned { continue }
        if let Some(index) = initializer_users.get(&(callee, 0)).into_iter().flatten()
            .find(|&&index| slot != 0 || matches!(initializer_uses[index].input, InitializerInput::Unproved))
        {
            return Err(format!("aggregate owning initializer receiver is not supported in {:?}", initializer_uses[*index].region.source));
        }
        let body = bodies.get(&callee).ok_or_else(|| format!("missing owning formal reservation {callee:?}"))?;
        let facts = body.facts.ok_or_else(|| format!("missing owning formal facts {callee:?}"))?;
        if !body.is_monomorphic || !body.supports_input_result || input_plans.contains_key(&callee) || outgoing.contains(&callee)
            || !facts.formals.contains_key(&slot) || facts.formal_unsupported.contains(&slot) || facts.raw_invalid.is_some() {
            return Err(format!("aggregate owning input has no closed formal in {:?} at slot {slot}", body.source));
        }
        for (point, _, slots) in facts.raw_uses.iter().filter(|(_, owner, _)| *owner == slot) {
            let raw = target_body(&declarations, body.cfg, *point).and_then(|wrapper| bodies.get(&wrapper))
                .filter(|wrapper| wrapper.is_monomorphic).and_then(|wrapper| wrapper.facts);
            let proved = raw.is_some_and(|raw| {
                raw.raw_invalid.is_none() && raw.raw_terminals.len() == 1
                    && raw.raw_terminals.iter().any(|(_, parameters)| parameters == slots)
            });
            if !proved {
                return Err(format!("aggregate owning formal has unproved transient raw route at point {point:?} in {:?}; wrapper raw use {:?}", body.source, raw.and_then(|facts| facts.raw_invalid)));
            }
        }
        let read_owner = ReadSource::Owner(slot);
        if facts.read_active.contains(&read_owner) {
            prove_read_views((callee, read_owner), &bodies, &declarations, &mut proven_read_views)?;
        }
        let origin = Some(OwnerSource::Formal(slot));
        let mut exits = facts.endpoints.iter().filter(|(_, owners)| owners.contains(&origin));
        let release = if let Some(&next) = forward_edges.get(&(callee, slot)) {
            if exits.next().is_some() {
                return Err(format!("aggregate owning formal has both release and transfer in {:?} at slot {slot}", body.source));
            }
            forward_predecessors.entry(next).or_default().push((callee, slot));
            None
        } else {
            let Some((&endpoint, observed)) = exits.next() else {
                return Err(format!("aggregate owning input has no proved formal exit in {:?} at slot {slot}", body.source));
            };
            if observed.len() != 1 || exits.next().is_some() {
                return Err(format!("aggregate owning input has ambiguous formal exit in {:?} at slot {slot}", body.source));
            }
            terminal_formals.push((callee, slot));
            Some(endpoint)
        };
        for (caller, ordinal, moved, origins) in users {
            if origins.is_empty() && !initializer_owned_formals.contains(&(callee, slot)) {
                continue;
            }
            let action = match origins.iter().next().filter(|_| origins.len() == 1) {
                Some(Some(InputOrigin::Plain(_))) => ClosedInput::Plain,
                Some(Some(InputOrigin::Formal(formal))) if matches!(moved, ArgumentProvenance::WholeMove(_))
                    && forward_edges.get(&(caller, *formal)) == Some(&(callee, slot))
                    && modes[&(caller, *formal)].0
                    && facts.raw_uses.is_empty()
                    && (ordinary_formal_outputs(&body.sites.boundary.root_outputs)
                        || (release.is_some()
                            && checked_scalar_output(&body.sites.boundary.root_outputs)
                            && facts.independent_checked_result
                            && bodies[&caller].facts.is_some_and(|facts| facts.checked_successor_result))) =>
                        ClosedInput::ForwardFormal(*formal),
                Some(source @ Some(InputOrigin::Owner(_))) if matches!(moved, ArgumentProvenance::WholeMove(_)) => {
                    let producer = active(caller, *source).ok_or_else(|| format!("aggregate owning input has unproved emitted incoming use in {:?} at call {ordinal} slot {slot}", bodies[&caller].source))?;
                    let call = bodies[&caller].facts.and_then(|f| f.calls.get(&producer)).ok_or_else(|| "missing owning input source receipt".to_owned())?;
                    if !discharged_by_inputs.insert((caller, producer)) {
                        return Err(format!("aggregate owning input has ambiguous source transfer in {:?} at call {ordinal}", bodies[&caller].source));
                    }
                    ClosedInput::Transfer(call.0)
                }
                Some(source @ Some(InputOrigin::Immediate(producer))) if moved == ArgumentProvenance::Immediate(*producer) => {
                    let caller_body = &bodies[&caller];
                    let facts = caller_body.facts.ok_or_else(|| "missing owning temporary facts".to_owned())?;
                    if active(caller, *source) != Some(*producer) {
                        return Err(format!("aggregate owning input has unproved emitted incoming use in {:?} at call {ordinal} slot {slot}", caller_body.source));
                    }
                    if let Some(exit) = facts.abandoned_temporaries.get(producer) {
                        return Err(format!("aggregate immediate argument has an unproved commit at {exit:?} in {:?}", caller_body.source));
                    }
                    if let Some(crossed) = facts.crossed_temporary_calls.get(producer) {
                        for &point in crossed {
                            let completion = target_body(&declarations, caller_body.cfg, point)
                                .and_then(|callee| bodies.get(&callee))
                                .is_some_and(|callee| callee.is_monomorphic && callee.supports_input_result
                                    && callee.facts.is_some_and(|facts| facts.returning_leaf));
                            if !completion {
                                return Err(format!("aggregate immediate argument crosses an unproved returning call at {point:?} in {:?}", caller_body.source));
                            }
                        }
                    }
                    let call = facts.calls.get(producer).ok_or_else(|| "missing immediate owning input receipt".to_owned())?;
                    if !discharged_by_inputs.insert((caller, *producer)) {
                        return Err(format!("aggregate owning input has ambiguous source transfer in {:?} at call {ordinal}", caller_body.source));
                    }
                    ClosedInput::Transfer(call.0)
                }
                _ => return Err(format!("aggregate owning input has unproved emitted incoming use in {:?} at call {ordinal} slot {slot}", bodies[&caller].source)),
            };
            if input_calls.entry(caller).or_default().insert(ordinal, (slot, action)).is_some() {
                return Err(format!("aggregate owning input has ambiguous source transfer in {:?} at call {ordinal}", bodies[&caller].source));
            }
        }
        input_plans.insert(callee, (slot, release, mixed));
    }
    while let Some(terminal) = terminal_formals.pop() {
        if let Some(predecessors) = forward_predecessors.remove(&terminal) {
            terminal_formals.extend(predecessors);
        }
    }
    if !forward_predecessors.is_empty() {
        return Err("aggregate owning formal transfer does not reach a proved release".into());
    }
    let is_closed_symbol = |source: &SourceIdentity, span| {
        declarations
            .get(&(source, span))
            .is_some_and(|&(module, item)| {
                let range = (module, item, None)..=(module, item, Some(usize::MAX));
                outgoing.range(range.clone()).next().is_some() || input_plans.range(range).next().is_some()
            })
    };
    for (region, escapes) in initializer_escapes {
        if escapes.iter().any(|declaration| is_closed_symbol(&declaration.source, declaration.range)) {
            return Err(format!("aggregate owning callable is used as initializer value in {:?}", region.source));
        }
    }
    let mut guarded_initializer = None;
    let mut initializer_plans = HashMap::<SourceIdentity, ClosedInitializer>::new();
    let mut initializer_producers = BTreeSet::new();
    for user in &initializer_uses {
        let target @ (module, item, _) = user.target;
        let range = (module, item, None)..=(module, item, Some(usize::MAX));
        if outgoing.range(range.clone()).next().is_none() && input_plans.range(range).next().is_none() {
            continue;
        }
        let receive = initializer_receives.get(&(&user.region.source, user.call))
            .and_then(|(body, slot)| input_plans.get(body).filter(|plan| plan.0 == *slot))
            .and_then(|plan| plan.1);
        let consume = input_plans.get(&target).filter(|plan| plan.0 == 0)
            .and_then(|plan| plan.1.map(|(exit, _)| (exit, plan.2)))
            .filter(|_| initializer_owned_formals.contains(&(target, 0))
                && !matches!(user.input, InitializerInput::Unproved)
                && bodies.get(&target).is_some_and(|body| body.sites.boundary.root_outputs.iter()
                    .all(|output| ordinary_value(&output.ty) || output.ty == ScalarType::Unit)));
        if receive.is_none() && consume.is_none() {
            return Err(format!("aggregate owning initializer receiver is not supported in {:?}", user.region.source));
        }
        guarded_initializer.get_or_insert(user.region);
        if consume.is_some() {
            let plan = initializer_plans.entry(user.region.source.clone()).or_default();
            let input = match user.input {
                InitializerInput::Owned(install, producer) => {
                    initializer_producers.insert(producer);
                    if plan.installs.insert(install.storage.range, install.producer.0).is_some() {
                        return Err(format!("duplicate closed initializer destination in {:?}", user.region.source));
                    }
                    ClosedInitializerInput::Consume(install.producer.0)
                }
                InitializerInput::Immediate(call, producer) => {
                    initializer_producers.insert(producer);
                    if plan.immediate.insert(call.0, user.call.0).is_some() {
                        return Err(format!("duplicate closed initializer immediate producer in {:?}", user.region.source));
                    }
                    ClosedInitializerInput::Immediate(call.0)
                }
                InitializerInput::Plain => ClosedInitializerInput::Plain,
                InitializerInput::Unproved => unreachable!("validated initializer input"),
            };
            if plan.inputs.insert(user.call.0, input).is_some() {
                return Err(format!("duplicate closed initializer input in {:?}", user.region.source));
            }
        }
    }

    for (&key, body) in &bodies {

        if body
            .sites
            .callable_escapes
            .iter()
            .any(|decl| is_closed_symbol(&decl.source, decl.range))
        {
            return Err(format!(
                "aggregate owning callable is used as a value in {:?}",
                body.source
            ));
        }
        let Some(facts) = body.facts else { continue };
        let returning = outgoing.contains(&key);
        let return_route = if returning {
            if !body.is_monomorphic {
                return Err(format!(
                    "aggregate owning generic reservation is not closed in {:?}",
                    body.source
                ));
            }
            let mut values = facts
                .returns
                .values()
                .flat_map(|alternatives| alternatives.iter().copied());
            let Some(Some(owner)) = values.next() else {
                return Err(format!(
                    "aggregate return owner is not established in {:?}",
                    body.source
                ));
            };
            if values.any(|candidate| candidate != Some(owner)) {
                return Err(format!(
                    "aggregate return owner has conflicting paths in {:?}",
                    body.source
                ));
            }
            let route = match owner {
                OwnerSource::Formal(_) => {
                    return Err(format!("return of moved formal is not closed in {:?}", body.source));
                }
                OwnerSource::Allocation(point) => {
                    let &(block, item) = facts.allocation_slots.get(&point).ok_or_else(|| {
                        format!("missing aggregate allocation binding in {:?}", body.source)
                    })?;
                    ClosedReturn::Allocation { block, item }
                }
                OwnerSource::Call(point) => {
                    let call = *facts.calls.get(&point).ok_or_else(|| {
                        format!(
                            "missing aggregate forwarding occurrence in {:?}",
                            body.source
                        )
                    })?;
                    ClosedReturn::Forward { call }
                }
            };
            Some(route)
        } else {
            None
        };
        let mut plan = result.bodies.remove(&key).unwrap_or_default();
        plan.returning = return_route;
        plan.input_calls = input_calls.remove(&key).unwrap_or_default();
        if let Some(&(slot, release, mixed)) = input_plans.get(&key) {
            plan.input = Some(slot);
            plan.input_mixed = mixed;
            if let Some((exit, block)) = release {
                plan.input_release = Some(exit);
                plan.scope_exits.insert(block, exit);
            }
        }
        for (&point, &call) in &facts.calls {
            if !target_body(&declarations, body.cfg, point)
                .is_some_and(|target| outgoing.contains(&target))
            {
                continue;
            }
            if matches!(return_route, Some(ClosedReturn::Forward { call: forwarded }) if call == forwarded)
            {
                plan.calls.insert(call.0, ClosedCall::Forward);
                continue;
            }
            if discharged_by_inputs.contains(&(key, point)) {
                if facts.endpoints.values().any(|owners| owners.contains(&Some(OwnerSource::Call(point)))) {
                    return Err(format!("aggregate input transfer does not cover every owning path in {:?}", body.source));
                }
                plan.calls.insert(call.0, ClosedCall::Receive);
                continue;
            }
            let mut endpoints = facts
                .endpoints
                .iter()
                .filter(|(_, alternatives)| alternatives.contains(&Some(OwnerSource::Call(point))));
            let Some((&(exit, block), alternatives)) = endpoints.next() else {
                return Err(format!(
                    "aggregate owning call has no proved endpoint in {:?}",
                    body.source
                ));
            };
            if alternatives.len() != 1 || endpoints.next().is_some() {
                return Err(format!(
                    "aggregate owning call has ambiguous endpoint in {:?}",
                    body.source
                ));
            }
            plan.calls.insert(call.0, ClosedCall::Receive);
            if plan.scope_exits.insert(block, exit).is_some_and(|old| old != exit) {
                return Err(format!("aggregate release has conflicting scope exits in {:?}", body.source));
            }
            plan.releases.entry(exit).or_default().push(call.0);
        }
        if (returning || !plan.calls.is_empty() || plan.input.is_some() || !plan.input_calls.is_empty()) && !body.is_monomorphic {
            return Err(format!("aggregate ownership in generic reservation {key:?} is not closed in {:?}", body.source));
        }
        if (returning || !plan.calls.is_empty() || plan.input.is_some() || !plan.input_calls.is_empty()) && !facts.unsupported.is_empty() {
            return Err(format!(
                "aggregate transfer has unsupported use in {:?} at point {:?}",
                body.source,
                facts.unsupported.first()
            ));
        }
        if returning || !plan.calls.is_empty() || plan.input.is_some() || !plan.input_calls.is_empty()
            || !plan.same_array_rebinds.is_empty()
        {
            result.bodies.insert(key, plan);
        }
    }
    if let Some(region) = guarded_initializer {
        if initializer_producers.iter().any(|producer| result.bodies.get(producer).is_none_or(|body| body.returning.is_none())) {
            return Err(format!("aggregate owning initializer receiver is not supported in {:?}", region.source));
        }
        result.initializers = initializer_plans;
    }
    Ok(result)
}

pub(in crate::scalar) fn close_installed(
    reservations: &AggregateReservations,
    program: &ScalarProgram,
    calls: &ModuleCallSites,
    facts: &InstalledAggregateFacts,
) -> Result<ClosedAggregates, String> {
    closed_aggregates(
        reservations,
        |module| (module == 0).then_some(ModuleInput { source: &program.source, items: &program.items, calls }),
        |(_, item, arm), function| Some((facts.0.get(&(item, arm)), &function.reference_cfg)),
        std::iter::once(&calls.initialization),
    )
}

impl ProjectReferenceResults {
    pub(in crate::scalar) fn closed_aggregates(
        &self,
        project: &ScalarProject,
        calls: &ProjectCallSiteResults,
        reservations: &AggregateReservations,
    ) -> Result<ClosedAggregates, String> {
        closed_aggregates(
            reservations,
            |module| {
                let row = project.modules.get(module)?;
                let sites = calls.modules.get(module)?.as_ref().ok()?;
                Some(ModuleInput { source: &row.source, items: &row.items, calls: sites })
            },
            |(module, item, arm), _| self.function_result(module, item, arm).map(|result| (Some(&result.aggregate), &result.cfg)),
            reservations.modules.iter().filter_map(|module| calls.modules.get(*module)?.as_ref().ok().map(|module| &module.initialization)),
        )
    }
}
