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
    pub(super) receipt_loan_sinks: BTreeMap<CfgPointId, Vec<CfgPointId>>,
    pub(super) receipt_loan_address_sinks: BTreeMap<CfgPointId, ReferenceBindingId>,
    pub(super) receipt_loan_forwarding: BTreeMap<CfgPointId, Vec<CfgPointId>>,
    pub(super) raw_formals: BTreeMap<usize, ReferenceBindingId>,
    pub(super) raw_calls: BTreeMap<CfgPointId, (Vec<usize>, bool)>,
    pub(super) read_consumers: BTreeSet<CfgPointId>,
    pub(super) read_fields: BTreeMap<usize, ScalarStructFieldId>,
    pub(super) read_calls: BTreeMap<CfgPointId, ReadCall>,
    pub(super) raw_nulls: BTreeSet<CfgPointId>,
    pub(super) plain_aliases: BTreeMap<CfgPointId, ReferenceBindingId>,
    pub(super) literals: BTreeMap<CfgPointId, Vec<(OccurrenceOrdinal, CfgPointId)>>,
    pub(super) installations: BTreeMap<CfgPointId, (CfgPointId, BlockPosition)>,
    receipt_aliases: BTreeSet<CfgPointId>,
    pub(super) calls: BTreeMap<CfgPointId, OccurrenceOrdinal>,
    pub(super) projected_reads: BTreeMap<CfgPointId, bool>,
    pub(super) scalar_receipt_reads: BTreeSet<CfgPointId>,
    pub(super) scope_exits: BTreeMap<CfgPointId, BlockOrdinal>,
    pub(super) formals: BTreeMap<usize, ReferenceBindingId>,
    pub(super) input_moves: BTreeMap<CfgPointId, (OccurrenceOrdinal, BTreeMap<usize, ArgumentProvenance>)>,
    pub(super) temporary_arguments: BTreeSet<CfgPointId>,
    pub(super) formal_forward_shape: Option<(CfgPointId, CfgPointId)>,
    pub(super) array_formal: Option<ReferenceBindingId>,
    pub(super) array_calls: BTreeMap<CfgPointId, (OccurrenceOrdinal, CfgPointId, ReferenceBindingId)>,
    // Primary return value and the whole-place move that follows its evaluation.
    pub(super) array_return: Option<(CfgPointId, CfgPointId)>,
    pub(super) conditional_array_return: Option<ConditionalArrayReturn>,
    pub(super) array_installs: BTreeMap<CfgPointId, OccurrenceOrdinal>,
    pub(super) neutral_array_calls: BTreeSet<CfgPointId>,
    pub(super) neutral_array_projections: BTreeMap<CfgPointId, CfgPointId>,
    pub(super) neutral_array_scalars: BTreeSet<ReferenceBindingId>,
    pub(super) local_array_copy_allocation: Option<CfgPointId>,
    pub(super) local_array_nulls: BTreeSet<CfgPointId>,
    pub(super) local_array_failure_arms: BTreeMap<CfgPointId, BlockOrdinal>,
    pub(super) checked_producer_arms: Vec<(CfgPointId, CheckedProducerTerminal)>,
    pub(super) checked_nullable_literals: BTreeMap<CfgPointId, ReferenceBindingId>,
    pub(super) checked_raw_null_initializers: BTreeSet<CfgPointId>,
    pub(super) passive_checked_comparisons: BTreeMap<CfgPointId, (CfgPointId, bool)>,
    pub(super) checked_raw_bindings: BTreeSet<CfgPointId>,
    pub(super) checked_backing_free_literals: BTreeSet<CfgPointId>,
    pub(super) checked_producer_nonlocal_installs: BTreeSet<CfgPointId>,
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
                    if self.local_array_copy_allocation.is_some()
                        && matches!(ty, ScalarType::RawPointer(_)) && self.checked_raw_null_initializers.contains(&initializer)
                    {
                        self.checked_raw_bindings.insert(point);
                    }
                    self.installations.insert(point, (initializer, position));
                }
            }
            ScalarType::CheckedReference { .. } if !allocation => {
                if let Some(initializer) = initializer {
                    self.installations.insert(point, (initializer, position));
                    self.receipt_aliases.insert(point);
                }
            }
            _ => {}
        }
    }
}

pub(super) struct ConditionalArrayReturn {
    pub(super) join: CfgPointId,
    pub(super) arms: [ConditionalArrayArm; 2],
}

pub(super) struct ConditionalArrayArm {
    pub(super) exit: CfgPointId,
    pub(super) moved: CfgPointId,
    pub(super) tail_call: Option<CfgPointId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum OwnerSource {
    Allocation(CfgPointId),
    Call(CfgPointId),
    Formal(usize),
}

pub(super) enum CheckedProducerTerminal {
    Null(CfgPointId),
    Fresh { borrow: CfgPointId, descriptor: ReferenceBindingId },
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CheckedProducerBacking {
    NoDescriptor,
    FreshWithoutBacking,
    FreshWithAllocation(CfgPointId),
    FreshWithNullableAllocation(CfgPointId),
    FreshWithExplicitNullAndLiveAllocation(CfgPointId),
    Unknown,
}

impl CheckedProducerBacking {
    fn proven_allocation(self) -> Option<CfgPointId> {
        match self {
            Self::FreshWithAllocation(base) | Self::FreshWithNullableAllocation(base)
            | Self::FreshWithExplicitNullAndLiveAllocation(base) => Some(base),
            Self::NoDescriptor | Self::FreshWithoutBacking | Self::Unknown => None,
        }
    }
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
    Carrier(usize),
    Formal(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Value {
    ScalarCell,
    RawFormal(usize),
    ReadOffset(ReadSource),
    RawCell { point: CfgPointId, owner: Option<usize> },
    Plain(CfgPointId),
    CheckedRawNull,
    CheckedNullDescriptor,
    View(OwnerSource),
    // A checked loan of a receipt's descriptor does not own the backing.
    ReceiptLoan(CfgPointId),
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

pub(super) fn array_neutral_scalar(ty: &ScalarType) -> bool {
    ordinary_value(ty) || matches!(ty, ScalarType::Enum(_))
}

pub(super) fn neutral_array_signature(parameters: &[ScalarType], outputs: &ScalarOutputSequence) -> bool {
    array_roundtrip_signature(parameters, outputs)
        && matches!(parameters.first(), Some(ScalarType::RuntimeArray { element, .. }) if ordinary_value(element))
        && parameters[1..].iter().all(array_neutral_scalar)
}

fn neutral_scalar_evaluation(value: &ScalarExpression) -> bool {
    matches!(value, ScalarExpression::Boolean { .. } | ScalarExpression::Char { .. }
        | ScalarExpression::Integer { .. } | ScalarExpression::Float { .. }
        | ScalarExpression::Member { enum_tag: Some(_), .. }
        | ScalarExpression::Binary { .. } | ScalarExpression::Unary { .. })
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

pub(in crate::scalar) fn ordinary_value(ty: &ScalarType) -> bool {
    matches!(ty, ScalarType::Bool | ScalarType::Char | ScalarType::I8 | ScalarType::I16 | ScalarType::I32 |
        ScalarType::I64 | ScalarType::I128 | ScalarType::U8 | ScalarType::U16 | ScalarType::U32 |
        ScalarType::U64 | ScalarType::U128 | ScalarType::F32 | ScalarType::F64)
}

#[derive(Clone, Default, PartialEq)]
pub(super) struct AggregateEdgeState {
    values: BTreeMap<CfgPointId, Value>,
    earliest_value_use: Option<usize>,
    bindings:
        BTreeMap<ReferenceBindingId, (Option<Value>, ReferenceScopeId, Option<BlockPosition>)>,
    moved: BTreeSet<OwnerSource>,
    input_moves: BTreeMap<CfgPointId, InputOrigin>,
    pending_temporaries: BTreeMap<CfgPointId, ReferenceScopeId>,
    array_formal_live: bool,
    array_formal_pending: Option<CfgPointId>,
    array_return_forwarded: bool,
    array_conditional_output: Option<CfgPointId>,
    array_tail_dependency: Option<CfgPointId>,
    array_dependencies: BTreeSet<CfgPointId>,
    local_array: Option<LocalArrayLife>,
}

#[derive(Clone, Default, PartialEq)]
struct LocalArrayLife {
    live: bool,
    pending: Option<CfgPointId>,
    invalid: bool,
    has_rebind: bool,
    viewed: bool,
    checked_producer_conflict: bool,
    checked_borrow: Option<(CfgPointId, CheckedProducerBacking)>,
}

impl AggregateEdgeState {
    fn holds_receipt(&self, call: CfgPointId) -> bool {
        let owner = OwnerSource::Call(call);
        !self.moved.contains(&owner)
            && self.bindings.values().any(|(value, _, _)| {
                matches!(value, Some(Value::Aggregate { backing, .. }) if *backing == owner)
            })
    }

    pub(super) fn expire_before(&mut self, successor: CfgPointId, last_use: &[usize]) {
        if self.earliest_value_use.is_none_or(|deadline| deadline >= successor.0) {
            return;
        }
        let mut earliest = None::<usize>;
        self.values.retain(|producer, _| {
            let deadline = last_use[producer.0];
            if deadline < successor.0 {
                return false;
            }
            earliest = Some(earliest.map_or(deadline, |old| old.min(deadline)));
            true
        });
        self.earliest_value_use = earliest;
    }

    fn record_value(&mut self, id: CfgPointId, value: Option<Value>, deadlines: Option<&[usize]>) {
        if let Some(deadlines) = deadlines {
            if let Some(value) = value {
                let deadline = deadlines[id.0];
                if deadline > id.0 {
                    let previous = self.values.insert(id, value);
                    debug_assert!(previous.is_none());
                    self.earliest_value_use = Some(self.earliest_value_use.map_or(deadline, |old| old.min(deadline)));
                }
            }
        } else if let Some(value) = value {
            self.values.insert(id, value);
        } else {
            self.values.remove(&id);
        }
    }

    fn forget_plain(&mut self, binding: &ReferenceBindingId) {
        if let Some((value @ Some(Value::Plain(_) | Value::CheckedRawNull | Value::CheckedNullDescriptor), _, _)) = self.bindings.get_mut(binding) {
            *value = None;
        }
    }
}

type LiveReceipt = (OwnerSource, ReferenceBindingId);

#[derive(Default)]
struct ScopeEndpoint {
    possible: BTreeSet<LiveReceipt>,
    // None means no path has been observed, not a path with no live receipts.
    always: Option<BTreeSet<LiveReceipt>>,
}

impl ScopeEndpoint {
    fn observe(&mut self, live: BTreeSet<LiveReceipt>) {
        self.possible.extend(live.iter().cloned());
        if let Some(always) = &mut self.always {
            always.retain(|receipt| live.contains(receipt));
        } else {
            self.always = Some(live);
        }
    }

    fn receipt(&self, owner: OwnerSource) -> Option<bool> {
        let mut bindings = self.possible.iter().filter(|(source, _)| *source == owner);
        let receipt = bindings.next()?;
        Some(bindings.next().is_none()
            && self.always.as_ref().is_some_and(|always| always.contains(receipt)))
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ArrayReturnPath {
    dependencies: BTreeSet<CfgPointId>,
    tail_call: Option<CfgPointId>,
}

#[derive(Default)]
pub(super) struct AggregateFlowResult {
    pub(super) checked_backing_forward_candidate: Option<(OccurrenceOrdinal, CfgPointId, CfgPointId)>,
    pub(super) checked_backing_forward: Option<(OccurrenceOrdinal, CfgPointId)>,
    pub(super) checked_backing_assignment: Option<(usize, usize, CfgPointId)>,
    pub(super) passive_descriptor_read: Option<Box<PassiveDescriptorRead>>,
    pub(super) checked_producer_blocks: BTreeMap<CfgPointId, usize>,
    pub(super) scalar_descriptor_assignment: Option<(usize, usize, usize)>,
    pub(super) checked_producer_backing: BTreeMap<CfgPointId, CheckedProducerBacking>,
    pub(super) checked_producer_sources: Vec<(CfgPointId, CheckedProducerTerminal)>,
    pub(super) checked_producer_authoritative: bool,
    pub(super) array_invalid: bool,
    pub(super) array_neutral: bool,
    array_conditional_forwarded: bool,
    pub(super) array_calls: BTreeMap<CfgPointId, (OccurrenceOrdinal, CfgPointId, ReferenceBindingId)>,
    pub(super) array_returns: BTreeSet<Option<ArrayReturnPath>>,
    pub(super) array_rebinds: BTreeMap<CfgPointId, (CfgPointId, ReferenceBindingId, OccurrenceOrdinal)>,
    pub(super) local_array_failure_arms: BTreeMap<usize, Option<CfgPointId>>,
    pub(super) allocation_slots: BTreeMap<CfgPointId, (usize, usize)>,
    pub(super) calls: BTreeMap<CfgPointId, OccurrenceOrdinal>,
    pub(super) returns: BTreeMap<CfgPointId, BTreeSet<Option<OwnerSource>>>,
    endpoints: BTreeMap<(CfgPointId, usize), ScopeEndpoint>,
    pub(super) unsupported: BTreeSet<CfgPointId>,
    unproved_receipt_loans: BTreeMap<CfgPointId, CfgPointId>,
    abandoned_temporaries: BTreeMap<CfgPointId, CfgPointId>,
    crossed_temporary_calls: BTreeMap<CfgPointId, BTreeSet<CfgPointId>>,
    pub(super) returning_leaf: bool,
    pub(super) checked_result_origins: BTreeSet<ReferenceOriginId>,
    pub(super) independent_checked_result: bool,
    pub(super) raw_read_forward_candidate: Option<CfgPointId>,
    pub(super) raw_read_forward_contract: bool,
    pub(super) raw_read_forward_result: bool,
    pub(super) formals: BTreeMap<usize, ReferenceBindingId>,
    pub(super) formal_unsupported: BTreeSet<usize>,
    pub(super) formal_forward: Option<(usize, CfgPointId, usize, usize)>,
    pub(super) checked_successor_candidate: Option<CfgPointId>,
    pub(super) checked_successor_contract: bool,
    pub(super) checked_successor_result: bool,
    raw_invalid: Option<CfgPointId>,
    pub(super) read_formals: BTreeSet<usize>,
    pub(super) read_formal_fields: BTreeMap<usize, ScalarStructFieldId>,
    read_invalid: BTreeMap<ReadSource, CfgPointId>,
    read_loads: BTreeSet<ReadSource>,
    read_active: BTreeSet<ReadSource>,
    read_edges: BTreeMap<ReadSource, BTreeSet<(CfgPointId, usize)>>,
    raw_terminals: BTreeSet<(CfgPointId, Vec<usize>)>,
    raw_uses: BTreeSet<(CfgPointId, usize, Vec<usize>)>,
    pub(super) inputs: BTreeMap<(CfgPointId, usize, usize, ArgumentProvenance), BTreeSet<Option<InputOrigin>>>,

}

impl AggregateFlowResult {
    fn reject_receipt_loan(&mut self, point: CfgPointId, value: Option<&Value>) -> bool {
        if let Some(Value::ReceiptLoan(call)) = value {
            self.unproved_receipt_loans.entry(*call).or_insert(point);
            true
        } else {
            false
        }
    }

    fn owner_endpoints(&self, owner: OwnerSource) -> impl Iterator<Item = ((CfgPointId, usize), bool)> + '_ {
        self.endpoints.iter().filter_map(move |(&endpoint, receipts)| {
            receipts.receipt(owner).map(|always| (endpoint, always))
        })
    }

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
        (self.checked_producer_authoritative && !self.checked_producer_backing.is_empty())
            || self.checked_backing_assignment.is_some() || self.checked_backing_forward.is_some()
            || self.scalar_descriptor_assignment.is_some() || !self.array_returns.is_empty() || !self.array_rebinds.is_empty()
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
    pub(super) fn value_deadlines<'a>(&self, ordinary_last_use: &'a [usize]) -> std::borrow::Cow<'a, [usize]> {
        let mut deadlines = std::borrow::Cow::Borrowed(ordinary_last_use);
        let mut record = |producer: CfgPointId, consumer: CfgPointId| {
            if deadlines[producer.0] < consumer.0 {
                deadlines.to_mut()[producer.0] = consumer.0;
            }
        };
        for (&consumer, inputs) in self.opaque_container_inputs.iter().chain(&self.receipt_loan_sinks) {
            for &producer in inputs {
                record(producer, consumer);
            }
        }
        for (&consumer, inputs) in &self.literals {
            for &(_, producer) in inputs {
                record(producer, consumer);
            }
        }
        for (&consumer, &(producer, _)) in &self.installations {
            record(producer, consumer);
        }
        for (&consumer, producers) in &self.receipt_loan_forwarding {
            for &producer in producers {
                record(producer, consumer);
            }
        }
        deadlines
    }

    pub(super) fn entry_state(&self) -> AggregateEdgeState {
        let mut state = AggregateEdgeState {
            array_formal_live: self.array_formal.is_some(),
            local_array: (self.local_array_copy_allocation.is_some()
                && (!self.local_array_failure_arms.is_empty() || !self.checked_producer_arms.is_empty()))
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
            || !self.checked_producer_arms.is_empty() || !self.views.is_empty() || !self.calls.is_empty() || !self.formals.is_empty() || !self.input_moves.is_empty() || !self.raw_formals.is_empty()
    }

    pub(super) fn visit(
        &self,
        point: &ReferenceFlowPoint,
        state: &mut AggregateEdgeState,
        result: &mut AggregateFlowResult,
        sites: &FunctionCallSites,
        value_deadlines: Option<&[usize]>,
    ) {
        let id = point.id;
        if let Some(local) = &mut state.local_array {
            let (binding, &allocation) = self.allocations.first_key_value().expect("local array demand");
            if id == allocation {
                local.invalid |= local.live || local.pending.is_some() || point.scope != ReferenceScopeId(0);
                local.live = true;
            }
            let view = self.views.get(&id) == Some(&allocation);
            if view {
                if self.checked_producer_arms.is_empty() {
                    local.invalid = true;
                } else {
                    local.checked_producer_conflict |= local.viewed;
                    local.viewed = true;
                }
            }
            let selected_recommit = |call| {
                self.array_calls.get(&call).is_some_and(|(_, moved, actual)| {
                    actual == binding && Some(*moved) == local.pending && !local.viewed
                })
            };
            if !self.checked_producer_arms.is_empty() {
                local.checked_producer_conflict |= self.checked_producer_nonlocal_installs.contains(&id);
                local.checked_producer_conflict |= match &point.kind {
                    CfgPointKind::Call { output: 0, target: ResolvedCallTarget::Core(CoreOperationId::IntExtend), .. } => false,
                    CfgPointKind::Call { output: 0, .. } => !self.neutral_array_calls.contains(&id) || !selected_recommit(id),
                    CfgPointKind::Call { .. } => !self.neutral_array_projections.get(&id)
                        .is_some_and(|&primary| selected_recommit(primary)),
                    CfgPointKind::AssignThrough { .. }
                    | CfgPointKind::MoveThrough { .. } | CfgPointKind::DeferredMoveThrough { .. }
                    | CfgPointKind::Invalidate { .. } | CfgPointKind::Release { .. }
                    | CfgPointKind::Evaluate(ScalarExpression::ArrayLiteral { .. }) => true,
                    CfgPointKind::Assign { target, rhs_point, .. } => {
                        !target.projections.is_empty()
                            || if &target.binding == binding {
                                !self.array_installs.contains_key(&id) || !selected_recommit(*rhs_point)
                            } else if self.neutral_array_scalars.contains(&target.binding) {
                                false
                            } else {
                                !state.bindings.contains_key(&target.binding)
                                    || self.views.get(rhs_point) != Some(&allocation)
                                    || state.values.get(rhs_point) != Some(&Value::View(OwnerSource::Allocation(allocation)))
                            }
                    }
                    CfgPointKind::Bind { .. } => self.installations.get(&id).is_some_and(|(rhs, _)| {
                        state.values.get(rhs) == Some(&Value::View(OwnerSource::Allocation(allocation)))
                    }),
                    CfgPointKind::Move { place, owner, .. } | CfgPointKind::DeferredMove { place, owner, .. } => {
                        self.checked_producer_arms.iter().any(|(_, terminal)| matches!(terminal,
                            CheckedProducerTerminal::Fresh { descriptor, .. }
                                if descriptor == owner || *descriptor == place.binding))
                    }
                    CfgPointKind::Evaluate(ScalarExpression::RawAddress { .. }) => !view,
                    _ => false,
                };
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
                    if self.array_installs.contains_key(&id) && self.array_calls.get(rhs_point)
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
                let witness = (local.live && !local.invalid && !local.viewed
                    && local.pending.is_none() && local.has_rebind)
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
            if let Some((_, CheckedProducerTerminal::Fresh { descriptor, .. })) = self.checked_producer_arms.iter().find(
                |(_, terminal)| matches!(terminal, CheckedProducerTerminal::Fresh { borrow, .. } if *borrow == id),
            ) {
                let backing = match state.bindings.get(descriptor).and_then(|(value, _, _)| *value) {
                    Some(Value::Aggregate { backing: OwnerSource::Allocation(base), field: Some(_) })
                        if local.live && local.viewed && base == allocation =>
                        CheckedProducerBacking::FreshWithAllocation(base),
                    Some(Value::Plain(origin)) if !local.viewed && self.checked_backing_free_literals.contains(&origin) =>
                        CheckedProducerBacking::FreshWithoutBacking,
                    Some(Value::CheckedNullDescriptor) if local.live && !local.viewed =>
                        CheckedProducerBacking::FreshWithExplicitNullAndLiveAllocation(allocation),
                    _ => CheckedProducerBacking::Unknown,
                };
                if local.checked_borrow.replace((id, backing)).is_some() {
                    local.checked_producer_conflict = true;
                }
            }
            if let Some((_, terminal)) = self.checked_producer_arms.iter().find(|(end, _)| *end == id) {
                let backing = if local.invalid || local.checked_producer_conflict || local.pending.is_some() {
                    CheckedProducerBacking::Unknown
                } else {
                    match terminal {
                        CheckedProducerTerminal::Null(_) if !local.viewed => CheckedProducerBacking::NoDescriptor,
                        CheckedProducerTerminal::Fresh { borrow, .. } => {
                            match local.checked_borrow {
                                Some((source, backing)) if source == *borrow => backing,
                                _ => CheckedProducerBacking::Unknown,
                            }
                        }
                        _ => CheckedProducerBacking::Unknown,
                    }
                };
                result.checked_producer_backing.entry(id)
                    .and_modify(|known| {
                        use CheckedProducerBacking::*;
                        *known = match (*known, backing) {
                            (left, right) if left == right => left,
                            (FreshWithAllocation(a) | FreshWithNullableAllocation(a), FreshWithExplicitNullAndLiveAllocation(b))
                            | (FreshWithExplicitNullAndLiveAllocation(a) | FreshWithNullableAllocation(a), FreshWithAllocation(b))
                            | (FreshWithAllocation(a) | FreshWithExplicitNullAndLiveAllocation(a), FreshWithNullableAllocation(b))
                                if a == b => FreshWithNullableAllocation(a),
                            _ => Unknown,
                        };
                    })
                    .or_insert(backing);
            }
        }
        if let (CfgPointKind::Assign { target, rhs_point, .. }, Some(&destination)) =
            (&point.kind, self.array_installs.get(&id))
        {
            if target.projections.is_empty() && self.array_calls.get(rhs_point)
                .is_some_and(|(_, _, binding)| *binding == target.binding)
            {
                result.array_rebinds.insert(id, (*rhs_point, target.binding.clone(), destination));
            }
        }
        if let Some(formal) = &self.array_formal {
            if let Some(conditional) = &self.conditional_array_return {
                if let Some(arm) = conditional.arms.iter().find(|arm| arm.exit == id) {
                    let verified = state.array_formal_pending == Some(arm.moved);
                    state.array_conditional_output = verified.then_some(id);
                    state.array_tail_dependency = arm.tail_call.filter(|_| verified);
                    if let Some(call) = state.array_tail_dependency {
                        // A tail call consumes a proof obligation, not a grounding path.
                        state.array_dependencies.insert(call);
                    }
                } else if id == conditional.join {
                    state.array_conditional_output = conditional.arms.iter()
                        .any(|arm| state.array_conditional_output == Some(arm.exit)
                            && state.array_formal_pending == Some(arm.moved)
                            && state.array_tail_dependency == arm.tail_call)
                        .then_some(id);
                }
            }
            if result.array_neutral {
                result.array_neutral = match &point.kind {
                    CfgPointKind::Entry | CfgPointKind::ScopeEnter | CfgPointKind::ScopeExit
                    | CfgPointKind::Read { .. } | CfgPointKind::Branch { .. } | CfgPointKind::Join
                    | CfgPointKind::ReturnOutput { checked: false, .. } | CfgPointKind::Return
                    | CfgPointKind::ConditionalOutput { .. } => true,
                    CfgPointKind::Bind { binding } => self.neutral_array_scalars.contains(binding),
                    CfgPointKind::CaptureAssignmentPrevious { .. } => true,
                    CfgPointKind::ReadPlace { pointer_point: None, candidates, reads_origin: false, .. } =>
                        state.array_formal_live && matches!(candidates.as_slice(), [place]
                            if &place.binding == formal && matches!(place.projections.as_slice(), [ReferencePlaceProjection::Index(_)])),
                    CfgPointKind::Assign { target, rhs_point, .. } => target.projections.is_empty()
                        && (self.neutral_array_scalars.contains(&target.binding)
                            || (&target.binding == formal && self.array_installs.contains_key(&id)
                                && self.neutral_array_calls.contains(rhs_point))),
                    CfgPointKind::Call { output: 0, target: ResolvedCallTarget::Core(CoreOperationId::IntExtend), .. } => true,
                    CfgPointKind::Call { output: 0, .. } => self.neutral_array_calls.contains(&id)
                        && self.array_calls.get(&id).is_some_and(|(_, moved, binding)|
                            binding == formal && Some(*moved) == state.array_formal_pending),
                    CfgPointKind::Call { .. } => self.neutral_array_projections.get(&id)
                        .and_then(|primary| self.array_calls.get(primary))
                        .is_some_and(|(_, moved, binding)| binding == formal && Some(*moved) == state.array_formal_pending),
                    CfgPointKind::Move { place, owner, ty } => &place.binding == owner
                        && place.projections.is_empty()
                        && (owner == formal
                            || matches!(ty, ScalarType::Enum(_)) && self.neutral_array_scalars.contains(owner)),
                    CfgPointKind::Evaluate(value) => neutral_scalar_evaluation(value),
                    _ => false,
                };
            }
            // Conditional authority is limited to the existing effect-neutral
            // fragment; an unresolved alias, release or backing effect is unknown.
            if self.conditional_array_return.is_some() && !result.array_neutral {
                result.array_invalid = true;
            }
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
                    if self.array_installs.contains_key(&id) && self.array_calls.get(rhs_point)
                        .is_some_and(|(_, moved, binding)| binding == formal && Some(*moved) == state.array_formal_pending)
                    {
                        state.array_formal_pending = None;
                        state.array_formal_live = true;
                        state.array_dependencies.insert(*rhs_point);
                    } else {
                        result.array_invalid = true;
                    }
                }
                CfgPointKind::ReturnOutput { output: 0, value_point, .. } => {
                    let conditional = state.array_conditional_output == Some(*value_point);
                    result.array_conditional_forwarded |= conditional;
                    state.array_return_forwarded = conditional
                        || self.array_return.is_some_and(|(value, moved)|
                            value == *value_point && Some(moved) == state.array_formal_pending);
                }
                CfgPointKind::Return => {
                    // Every normal return must have carried the checked formal to lane zero.
                    // The pending formal move alone says nothing about a different output.
                    result.array_returns.insert(
                        state.array_return_forwarded.then(|| ArrayReturnPath {
                            dependencies: state.array_dependencies.clone(),
                            tail_call: state.array_tail_dependency,
                        })
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
            CfgPointKind::ReadPlace { pointer_point: Some(pointer), .. } => {
                let value = state.values.get(pointer);
                if let Some(Value::ReceiptLoan(call)) = value {
                    if !self.scalar_receipt_reads.contains(&id) || !state.holds_receipt(*call) {
                        result.reject_receipt_loan(id, value);
                    }
                }
                result.account_raw(id, value);
            }
            CfgPointKind::AssignThrough { pointer_point: pointer, .. }
            | CfgPointKind::Invalidate { address_point: pointer, .. }
            | CfgPointKind::Release { target: ReleaseTarget::Checked { address_point: pointer, .. } } => {
                let value = state.values.get(pointer);
                result.reject_receipt_loan(id, value);
                result.account_raw(id, value);
            }
            CfgPointKind::MoveThrough { pointer_point, .. }
            | CfgPointKind::DeferredMoveThrough { pointer_point, .. } => {
                result.reject_receipt_loan(id, state.values.get(pointer_point));
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
        if let Some(operands) = self.receipt_loan_sinks.get(&id) {
            for operand in operands {
                result.reject_receipt_loan(id, state.values.get(operand));
            }
        }
        if let Some(binding) = self.receipt_loan_address_sinks.get(&id) {
            result.reject_receipt_loan(id, state.bindings.get(binding).and_then(|(value, _, _)| value.as_ref()));
        }
        if let Some(binding) = self.plain_aliases.get(&id) {
            state.forget_plain(binding);
        }
        let value = if let Some(sources) = self.receipt_loan_forwarding.get(&id) {
            let mut loans = sources.iter().filter_map(|source| match state.values.get(source) {
                Some(loan @ Value::ReceiptLoan(_)) => Some(*loan),
                _ => None,
            });
            let loan = loans.next();
            for other in loans {
                if Some(other) != loan {
                    result.reject_receipt_loan(id, loan.as_ref());
                    result.reject_receipt_loan(id, Some(&other));
                }
            }
            loan
        } else if let Some((root, whole)) = self.raw_addresses.get(&id) {
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
                    Some(value @ Value::ReceiptLoan(_)) => {
                        result.reject_receipt_loan(id, Some(value));
                        continue;
                    }
                    Some(Value::Unknown) => Value::Unknown,
                    Some(Value::Plain(_) | Value::CheckedRawNull | Value::CheckedNullDescriptor | Value::ScalarCell) | None => continue,
                };
                member = Some(if let Some(previous) = member {
                    result.account_raw(id, Some(&previous));
                    result.account_raw(id, Some(&next));
                    Value::Unknown
                } else {
                    next
                });
            }
            if member.is_none() && !self.checked_producer_arms.is_empty() {
                if self.checked_nullable_literals.get(&id).is_some_and(|binding|
                    matches!(state.bindings.get(binding), Some((Some(Value::CheckedRawNull), _, _))))
                {
                    member = Some(Value::CheckedNullDescriptor);
                }
            }
            member
        } else {
            match &point.kind {
                CfgPointKind::Borrow { place, .. } => {
                    state.forget_plain(&place.binding);
                    let value = state.bindings.get(&place.binding).and_then(|(value, _, _)| value.as_ref());
                    result.account_raw(id, value);
                    if let Some(Value::Aggregate { backing: OwnerSource::Call(call), .. } | Value::ReceiptLoan(call)) = value {
                        if !state.holds_receipt(*call) {
                            result.reject_receipt_loan(id, Some(&Value::ReceiptLoan(*call)));
                        }
                        Some(Value::ReceiptLoan(*call))
                    } else {
                        None
                    }
                }
                CfgPointKind::Read { binding } => {
                    state.bindings.get(binding).and_then(|(value, _, _)| *value)
                        .filter(|value| !matches!(value, Value::CheckedRawNull | Value::CheckedNullDescriptor))
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
                            (Value::CheckedRawNull | Value::CheckedNullDescriptor, _) => None,
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
                        let value = if !self.checked_producer_arms.is_empty() && self.checked_raw_bindings.contains(&id) {
                            Some(Value::CheckedRawNull)
                        } else {
                            state.values.get(&rhs).copied().filter(|value| !matches!(value, Value::Plain(origin) if *origin != rhs))
                        };
                        if !self.receipt_aliases.contains(&id) || matches!(value, Some(Value::ReceiptLoan(_))) {
                            state.bindings.insert(binding.clone(), (value, point.scope, Some(position)));
                        }
                    }
                    None
                }
                CfgPointKind::Assign {
                    target, rhs_point, ..
                } => {
                    for value in state.bindings.get(&target.binding).and_then(|(value, _, _)| value.as_ref()).into_iter().chain(state.values.get(rhs_point)) {
                        result.account_raw(id, Some(value));
                    }
                    let rhs = state.values.get(rhs_point);
                    if result.reject_receipt_loan(id, rhs) {
                        // An assignment is not a supported proof of a local loan alias.
                    } else if target.projections.is_empty() {
                        let source = rhs.copied().filter(|value| !matches!(value, Value::Plain(_)));
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
                    } else if let Some((value @ Some(Value::Plain(_) | Value::CheckedRawNull | Value::CheckedNullDescriptor), _, _)) = state.bindings.get_mut(&target.binding) {
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
                    if !result.reject_receipt_loan(id, value) && value.is_some() {
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
                        if result.reject_receipt_loan(id, value) {
                            continue;
                        }
                        if ordinary_read.is_some() && matches!(value, Some(Value::RawFormal(_))) {
                            result.invalidate_raw(id);
                        }
                        if let Some(source) = value.and_then(|value| value.read_source()) {
                            let live = match source {
                                ReadSource::Formal(_) => true,
                                ReadSource::Owner(owner) | ReadSource::Carrier(owner) => !state.moved.contains(&OwnerSource::Formal(owner))
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
                        let receipt = result.reject_receipt_loan(id, Some(left))
                            | result.reject_receipt_loan(id, Some(right));
                        (!receipt).then_some(Value::Unknown)
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
                        Some(Value::ReceiptLoan(_)) => {
                            result.reject_receipt_loan(id, return_value);
                            None
                        }
                        Some(Value::View(_) | Value::Unknown) => {
                            result.unsupported.insert(id);
                            None
                        }
                        Some(Value::RawFormal(_) | Value::ReadOffset(_) | Value::RawCell { .. }) => None,
                        Some(Value::Plain(_) | Value::CheckedRawNull | Value::CheckedNullDescriptor | Value::ScalarCell) | None => None,
                    };
                    result.returns.entry(id).or_default().insert(returning);
                    None
                }
                CfgPointKind::ScopeExit => {
                    let exit = self.scope_exits.get(&id);
                    let mut live = BTreeSet::new();
                    state.bindings.retain(|binding, (value, owner_scope, position)| {
                        if *owner_scope != point.scope {
                            return true;
                        }
                        // A field-backed carrier can use an owner's storage but
                        // is not a whole Call or Formal receipt to release.
                        if let Some(Value::Aggregate { backing, field: None }) = value {
                            let installed = match backing {
                                OwnerSource::Formal(_) => true,
                                OwnerSource::Call(_) => position.is_some(),
                                OwnerSource::Allocation(_) => false,
                            };
                            if exit.is_some() && installed && !state.moved.contains(backing) {
                                live.insert((*backing, binding.clone()));
                            }
                        }
                        false
                    });
                    if let Some(exit) = exit {
                        result.endpoints.entry((id, exit.0)).or_default().observe(live);
                    }
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
        }
        state.record_value(id, value, value_deadlines);
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

/// Origin of the existing allocation, not the current value of its binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OriginalArrayReceipt {
    origin: CfgPointId,
    pub(crate) allocation: (usize, usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ClosedLocalArrayRelease {
    pub(crate) receipt: OriginalArrayReceipt,
    pub(crate) arm: usize,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ClosedCheckedBacking {
    pub(crate) receipt: OriginalArrayReceipt,
    pub(crate) arms: [usize; 2],
    pub(crate) failure: Option<usize>,
}

#[derive(Debug, Default)]
pub(crate) struct ClosedBody {
    pub(crate) scalar_descriptor_assignment: Option<(usize, usize, usize)>,
    pub(crate) checked_backing_producer: Option<ClosedCheckedBacking>,
    pub(crate) checked_backing_wrapper: Option<usize>,
    pub(crate) checked_backing_consumer: Option<(usize, usize)>,
    pub(crate) checked_backing_optional: bool,
    pub(crate) returning: Option<ClosedReturn>,
    pub(crate) ordinary_result: bool,
    pub(crate) ordinary_calls: BTreeMap<usize, SourceSpan>,
    pub(crate) calls: BTreeMap<usize, ClosedCall>,
    pub(crate) scope_exits: BTreeMap<usize, CfgPointId>,
    pub(crate) releases: BTreeMap<CfgPointId, Vec<usize>>,
    pub(crate) input: Option<usize>,
    pub(crate) input_mixed: bool,
    // Checked primary call occurrence -> exact destination place occurrence.
    pub(crate) same_array_rebinds: BTreeMap<usize, usize>,
    pub(crate) local_array_failure: Option<ClosedLocalArrayRelease>,
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
    let check_effects = matches!(root.1, ReadSource::Carrier(_));
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
        let outputs = &body.sites.boundary.root_outputs;
        let supported_outputs = outputs.iter().all(|output| ordinary_value(&output.ty) || output.ty == ScalarType::Unit)
            || (checked_scalar_output(outputs)
                && (facts.independent_checked_result || facts.raw_read_forward_result));
        let observations = match source {
            ReadSource::Formal(slot) if facts.read_formals.contains(&slot) && supported_outputs => source,
            ReadSource::Owner(_) => source,
            ReadSource::Carrier(slot) if supported_outputs
                && facts.read_formal_fields.contains_key(&slot)
                && !facts.formal_unsupported.contains(&slot) => ReadSource::Owner(slot),
            _ => return Err(format!("unproved ordinary raw read formal {source:?} at {key:?} in {:?}", body.source)),
        };
        if !body.is_monomorphic {
            return Err(format!("unproved ordinary raw read formal {source:?} at {key:?} in {:?}", body.source));
        }
        if let Some(point) = facts.read_invalid.get(&observations) {
            return Err(format!("unproved ordinary raw read use at {point:?} for {source:?} in {:?}", body.source));
        }
        if facts.read_loads.contains(&observations) {
            loads.push(node);
        }
        let raw_edges = facts.read_edges.get(&observations);
        let carrier_edge = match (source, facts.formal_forward) {
            (ReadSource::Carrier(source_slot), Some((formal, point, _, destination_slot)))
                if formal == source_slot => Some((point, destination_slot)),
            _ => None,
        };
        if check_effects && (!body.cfg.allocations.is_empty()
            || !body.sites.allocation_length_call_targets.is_empty()
            || body.cfg.points.iter().any(|call| {
                if matches!(&call.kind, CfgPointKind::Release { .. } | CfgPointKind::Invalidate { .. }
                    | CfgPointKind::AssignThrough { .. })
                {
                    return true;
                }
                let CfgPointKind::Call { output: 0, target, .. } = &call.kind else { return false };
                match target {
                    ResolvedCallTarget::Core(operation) => !matches!(operation,
                        CoreOperationId::Load | CoreOperationId::Offset | CoreOperationId::IntTrunc
                        | CoreOperationId::IntExtend | CoreOperationId::UintToFloat
                        | CoreOperationId::SintToFloat | CoreOperationId::FloatToSintTrunc
                        | CoreOperationId::FloatToUintTrunc | CoreOperationId::FloatTrunc
                        | CoreOperationId::FloatExtend | CoreOperationId::Bitcast),
                    ResolvedCallTarget::ModuleCallable { .. } => {
                        !carrier_edge.is_some_and(|(point, _)| point == call.id)
                            && !raw_edges.is_some_and(|edges| edges.range((call.id, 0)..=(call.id, usize::MAX)).next().is_some())
                            && !target_body(declarations, body.cfg, call.id)
                                .and_then(|target| bodies.get(&target))
                                .is_some_and(|callee| callee.is_monomorphic
                                    && callee.cfg.allocations.is_empty()
                                    && callee.sites.allocation_length_call_targets.is_empty()
                                    && callee.facts.is_some_and(|facts| facts.returning_leaf))
                    }
                    _ => true,
                }
            })) {
            return Err(format!("unproved effect in passive reader {source:?} at {key:?} in {:?}", body.source));
        }
        if let Some(edges) = raw_edges {
            for &(point, slot) in edges {
                let target = target_body(declarations, body.cfg, point)
                    .ok_or_else(|| format!("unbound ordinary raw read call at {point:?} in {:?}", body.source))?;
                let next = (target, ReadSource::Formal(slot));
                previous.entry(next).or_default().push(node);
                pending.push(next);
            }
        }
        if let (ReadSource::Carrier(source_slot), Some((point, destination_slot))) = (source, carrier_edge) {
            let target = target_body(declarations, body.cfg, point)
                .and_then(|target| bodies.get(&target).map(|callee| (target, callee)))
                .filter(|(_, callee)| callee.is_monomorphic && callee.facts.is_some_and(|callee_facts| {
                    callee_facts.read_formal_fields.get(&destination_slot)
                        == facts.read_formal_fields.get(&source_slot)
                }))
                .map(|(target, _)| target)
                .ok_or_else(|| format!("unproved exact carrier forwarding at {point:?} in {:?}", body.source))?;
            let next = (target, ReadSource::Carrier(destination_slot));
            previous.entry(next).or_default().push(node);
            pending.push(next);
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct OrdinaryResultSite {
    pub(crate) caller: BodyKey,
    pub(crate) call: OccurrenceOrdinal,
}

enum OrdinaryResultCandidate {
    Unused,
    Direct(OrdinaryResultSite),
    Refused,
}

impl OrdinaryResultCandidate {
    fn record(&mut self, site: Option<OrdinaryResultSite>) {
        *self = match (&*self, site) {
            (Self::Unused, Some(site)) => Self::Direct(site),
            _ => Self::Refused,
        };
    }
}

fn ordinary_result_literal(expression: &ScalarExpression) -> bool {
    let ScalarExpression::StructLiteral { fields, .. } = expression else { return false };
    fields.iter().all(|field| matches!(&field.value,
        ScalarExpression::Integer { .. } | ScalarExpression::Float { .. }
        | ScalarExpression::Boolean { .. } | ScalarExpression::Char { .. }))
}

fn live_ordinary_result_storage(block: &ScalarBlock, ty: &ScalarType, root: bool) -> bool {
    if block.final_output_values.len() != 1 || block.unsafe_context {
        return false;
    }
    match block.items.as_slice() {
        [ScalarBlockItem::Expression(ScalarExpression::If { condition, then_branch, else_branch, .. })] => {
            matches!(condition.as_ref(), ScalarExpression::Name { .. } | ScalarExpression::Boolean { .. })
                && live_ordinary_result_storage(then_branch, ty, false)
                && live_ordinary_result_storage(else_branch, ty, false)
        }
        [ScalarBlockItem::Expression(expression)] => root && ordinary_result_literal(expression),
        [ScalarBlockItem::LocalBinding(binding), ScalarBlockItem::Expression(ScalarExpression::Name { name, .. })] => {
            !binding.is_allocation
                && matches!(binding.receivers.as_slice(), [receiver] if receiver.name == *name && receiver.ty == *ty)
                && ordinary_result_literal(&binding.value)
        }
        _ => false,
    }
}

fn scalar_only_ordinary_result_use(
    function: &ScalarFunction,
    sites: &FunctionCallSites,
    call: OccurrenceOrdinal,
) -> bool {
    let ScalarType::Callable { parameters, outputs } = &function.signature else { return false };
    if !parameters.iter().all(ordinary_value)
        || !matches!(outputs.outputs.as_slice(), [output] if ordinary_value(&output.ty))
        || function.body.unsafe_context
        || function.body.final_output_values.len() != 1
    {
        return false;
    }
    let [ScalarBlockItem::LocalBinding(binding), ScalarBlockItem::Expression(ScalarExpression::Member { receiver, .. })] =
        function.body.items.as_slice() else { return false };
    !binding.is_allocation
        && matches!(binding.receivers.as_slice(), [value] if &value.name == receiver && matches!(value.ty, ScalarType::Struct(_)))
        && matches!(binding.value, ScalarExpression::Call { .. })
        && sites.structure.edges.get(&(
            OccurrenceParent::Block(sites.structure.root),
            OccurrenceEdge::BindingRoot { item: 0, output: 0 },
        )) == Some(&OccurrenceNode::Expression(call))
}

pub(super) fn ordinary_result_cohort<'a>(
    reservations: &AggregateReservations,
    module_at: impl Fn(usize) -> Option<(&'a SourceIdentity, &'a [ScalarItem], &'a [ScalarStruct], &'a ModuleCallSites)>,
) -> BTreeMap<BodyKey, OrdinaryResultSite> {
    let mut definitions = HashMap::new();
    let mut candidates = BTreeMap::new();
    for (&key @ (module, item, arm), &ordinary) in &reservations.bodies {
        if !ordinary || arm.is_some() {
            continue;
        }
        let Some((source, items, structs, _)) = module_at(module) else { return BTreeMap::new() };
        let Some(ScalarItem::Function(function)) = items.get(item) else { return BTreeMap::new() };
        if !crate::scalar::typecheck::is_module_private_name(&function.name)
            || !function.generic_parameters.is_empty()
            || !function.overload_arms.is_empty()
        {
            continue;
        }
        let ScalarType::Callable { parameters, outputs } = &function.signature else { continue };
        if !parameters.iter().all(ordinary_value) {
            continue;
        }
        let [ScalarOutput { ty, .. }] = outputs.outputs.as_slice() else { continue };
        let ScalarType::Struct(id) = ty else { continue };
        if !matches!(function.body.final_output_values.as_slice(), [output] if output.ty == *ty)
            || !live_ordinary_result_storage(&function.body, ty, true)
        {
            continue;
        }
        let Some(structure) = structs.iter().find(|structure| &structure.id == id) else { continue };
        if !structure.fields.iter().all(|field| ordinary_value(&field.ty)) {
            continue;
        }
        if definitions.insert((source, function.name_span), key).is_some() {
            return BTreeMap::new();
        }
        candidates.insert(key, OrdinaryResultCandidate::Unused);
    }
    if candidates.is_empty() {
        return BTreeMap::new();
    }
    let mut visit = |source: &SourceIdentity, range, site| {
        if let Some(key) = definitions.get(&(source, range)) {
            candidates.get_mut(key).expect("ordinary result candidate").record(site);
        }
    };
    for (&caller @ (module, item, arm), &ordinary) in &reservations.bodies {
        let Some((_, items, _, calls)) = module_at(module) else { return BTreeMap::new() };
        let Some(ScalarItem::Function(function)) = items.get(item) else { return BTreeMap::new() };
        let Some(sites) = calls.function_body(item, arm) else { return BTreeMap::new() };
        if !matches!(&sites.plain_names, PlainNameFacts::Prepared(_))
            || sites.allocation_length_calls.iter().any(|ordinal| !sites.allocation_length_call_targets.contains_key(ordinal))
        {
            return BTreeMap::new();
        }
        for escape in &sites.callable_escapes {
            visit(&escape.source, escape.range, None);
        }
        for target in sites.allocation_length_call_targets.values() {
            match target {
                ResolvedCallTarget::ModuleCallable { source, declaration_span, .. } => {
                    visit(source, *declaration_span, None);
                }
                ResolvedCallTarget::LocalCallable(_) | ResolvedCallTarget::CallableField { .. } => return BTreeMap::new(),
                _ => {}
            }
        }
        for occurrence in &sites.call_occurrences {
            let Some(resolved) = sites.sites.get(occurrence.site) else { return BTreeMap::new() };
            if matches!(&resolved.target, ResolvedCallTarget::LocalCallable(_) | ResolvedCallTarget::CallableField { .. }) {
                return BTreeMap::new();
            }
            if let ResolvedCallTarget::ModuleCallable { source, declaration_span, concrete } = &resolved.target {
                let allowed = ordinary && arm.is_none()
                    && function.generic_parameters.is_empty() && function.overload_arms.is_empty()
                    && matches!(concrete, ConcreteCallSelection::Declaration)
                    && scalar_only_ordinary_result_use(function, sites, occurrence.ordinal);
                visit(source, *declaration_span, allowed.then_some(OrdinaryResultSite {
                    caller,
                    call: occurrence.ordinal,
                }));
            }
        }
    }
    for &module in &reservations.modules {
        let Some((_, _, _, calls)) = module_at(module) else { return BTreeMap::new() };
        let ModuleInitializationDetail::Occurrences(topology) = &calls.initialization.detail else { return BTreeMap::new() };
        for escape in &topology.callable_escapes {
            visit(&escape.source, escape.range, None);
        }
        for &ordinal in &topology.calls {
            match calls.initialization.call_target(ordinal) {
                Some(ModuleInitializationCallTarget::Direct { declaration, .. }
                    | ModuleInitializationCallTarget::Generic { declaration, .. }) => {
                    visit(&declaration.source, declaration.range, None);
                }
                Some(ModuleInitializationCallTarget::CallableBinding(_) | ModuleInitializationCallTarget::CallableField { .. })
                | None => return BTreeMap::new(),
                Some(_) => {}
            }
        }
    }
    candidates.into_iter().filter_map(|(key, candidate)| match candidate {
        OrdinaryResultCandidate::Direct(site) if site.caller != key => Some((key, site)),
        _ => None,
    }).collect()
}

struct ModuleInput<'a> {
    source: &'a SourceIdentity,
    items: &'a [ScalarItem],
    structs: &'a [ScalarStruct],
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

enum CheckedBackingIncoming {
    Sink((usize, usize)),
    Wrapper(usize),
}

fn closed_checked_backing_consumer<'a>(
    reservations: &AggregateReservations,
    module_at: &impl Fn(usize) -> Option<ModuleInput<'a>>,
    bodies: &BTreeMap<BodyKey, Body<'_>>,
    declarations: &HashMap<(&SourceIdentity, ByteSpan), (usize, usize)>,
    initializers: impl Iterator<Item = &'a ModuleInitializationRegion>,
    producer: BodyKey,
) -> Result<Option<(BodyKey, CheckedBackingIncoming)>, String> {
    let input = module_at(producer.0).ok_or_else(|| "missing checked backing module".to_owned())?;
    let Some(ScalarItem::Function(definition)) = input.items.get(producer.1) else {
        return Err("missing checked backing declaration".into());
    };
    let same_declaration = |source: &SourceIdentity, span| source == input.source && span == definition.name_span;
    let mut consumer = None;
    for (&key @ (module, item, arm), &monomorphic) in &reservations.bodies {
        let input = module_at(module).ok_or_else(|| "missing checked backing user module".to_owned())?;
        let sites = input.calls.function_body(item, arm).ok_or_else(|| "missing checked backing user body".to_owned())?;
        if sites.callable_escapes.iter().any(|decl| same_declaration(&decl.source, decl.range)) {
            return Ok(None);
        }
        if sites.allocation_length_call_targets.values().any(|target| matches!(target,
            ResolvedCallTarget::ModuleCallable { source, declaration_span, .. } if same_declaration(source, *declaration_span)))
        {
            return Ok(None);
        }
        for call in &sites.call_occurrences {
            let Some(site) = sites.sites.get(call.site) else { return Ok(None) };
            let ResolvedCallTarget::ModuleCallable { source, declaration_span, concrete } = &site.target else { continue };
            if !same_declaration(source, *declaration_span) { continue }
            if consumer.is_some() || !monomorphic || key == producer || arm.is_some()
                || !sites.allocation_length_call_targets.is_empty()
                || !matches!(concrete, ConcreteCallSelection::Declaration)
            {
                return Ok(None);
            }
            let Some(body) = bodies.get(&key) else { return Ok(None) };
            let Some(facts) = body.facts else { return Ok(None) };
            let incoming = if let Some((destination, callee, point)) = facts.checked_backing_assignment {
                if callee != call.ordinal.0 || target_body(declarations, body.cfg, point) != Some(producer) {
                    return Ok(None);
                }
                if sites.call_occurrences.len() != 1 {
                    let Some(passive) = facts.passive_descriptor_read.as_ref() else { return Ok(None) };
                    let Some(reader) = target_body(declarations, body.cfg, passive.call) else { return Ok(None) };
                    if !bodies.get(&reader).and_then(|body| body.facts)
                        .and_then(|facts| facts.read_formal_fields.get(&0))
                        .is_some_and(|field| field == &passive.field)
                        || prove_read_views((reader, ReadSource::Carrier(0)), bodies, declarations, &mut BTreeSet::new()).is_err()
                    {
                        return Ok(None);
                    }
                }
                CheckedBackingIncoming::Sink((destination, callee))
            } else if let Some((callee, point)) = facts.checked_backing_forward {
                if sites.call_occurrences.len() != 1 || callee != call.ordinal
                    || target_body(declarations, body.cfg, point) != Some(producer)
                {
                    return Ok(None);
                }
                CheckedBackingIncoming::Wrapper(callee.0)
            } else {
                return Ok(None);
            };
            consumer = Some((key, incoming));
        }
    }
    if consumer.is_none() { return Ok(None) }
    for region in initializers {
        let ModuleInitializationDetail::Occurrences(topology) = &region.detail else { return Ok(None) };
        if topology.callable_escapes.iter().any(|decl| same_declaration(&decl.source, decl.range))
            || topology.calls.iter().any(|call| {
                match region.call_target(*call) {
                    Some(ModuleInitializationCallTarget::Direct { declaration, .. }
                        | ModuleInitializationCallTarget::Generic { declaration, .. }) =>
                        same_declaration(&declaration.source, declaration.range),
                    Some(_) => false,
                    None => true,
                }
            })
        {
            return Ok(None);
        }
    }
    Ok(consumer)
}

fn closed_aggregates<'a>(
    reservations: &AggregateReservations,
    module_at: impl Fn(usize) -> Option<ModuleInput<'a>>,
    result_at: impl Fn(BodyKey, &'a ScalarFunction) -> Option<(Option<&'a AggregateFlowResult>, &'a ScalarReferenceCfg)>,
    initializers: impl Iterator<Item = &'a ModuleInitializationRegion> + Clone,
) -> Result<ClosedAggregates, String> {
    let ordinary = ordinary_result_cohort(reservations, |module| {
        let input = module_at(module)?;
        Some((input.source, input.items, input.structs, input.calls))
    });
    let install_ordinary = |result: &mut ClosedAggregates| {
        for (&producer, site) in &ordinary {
            if result.bodies.contains_key(&producer) || result.bodies.contains_key(&site.caller) {
                continue;
            }
            let Some(input) = module_at(producer.0) else { continue };
            let Some(ScalarItem::Function(function)) = input.items.get(producer.1) else { continue };
            let declaration = SourceSpan::new(input.source.clone(), function.name_span);
            result.bodies.entry(producer).or_default().ordinary_result = true;
            result.bodies.entry(site.caller).or_default().ordinary_calls.insert(site.call.0, declaration);
        }
    };
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
    let mut result = ClosedAggregates::default();
    for (&key, body) in &bodies {
        if let Some(facts) = body.facts {
            if body.is_monomorphic {
                if let Some(proof) = facts.scalar_descriptor_assignment {
                    result.bodies.entry(key).or_default().scalar_descriptor_assignment = Some(proof);
                }
            }
            if body.is_monomorphic && !facts.array_rebinds.is_empty() {
                array_callsites.push((key, body.cfg, facts));
            }
            if body.is_monomorphic && !facts.array_invalid && !facts.array_returns.is_empty() {
                let mut grounded = false;
                let mut dependencies = BTreeSet::new();
                let valid = facts.array_returns.iter().all(|path| {
                    let Some(path) = path else { return false };
                    let mut self_call = false;
                    for &point in &path.dependencies {
                        let Some(target) = target_body(&declarations, body.cfg, point) else { return false };
                        if path.tail_call == Some(point) && target != key {
                            return false;
                        }
                        if facts.array_conditional_forwarded && target != key
                            && !bodies.get(&target).and_then(|callee| callee.facts)
                                .is_some_and(|callee| callee.array_neutral)
                        {
                            return false;
                        }
                        self_call |= target == key;
                        dependencies.insert(target);
                    }
                    grounded |= path.tail_call.is_none() && !self_call;
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

    for (key, cfg, facts) in array_callsites {
        for &(call, _, destination) in facts.array_rebinds.values() {
            let Some((ordinal, _, _)) = facts.array_calls.get(&call) else { continue };
            let Some(callee) = target_body(&declarations, cfg, call) else { continue };
            if array_leaves.contains(&callee)
                || array_neighbors.get(&callee).is_some_and(|deps| deps.is_subset(&array_leaves))
            {
                let previous = result.bodies.entry(key).or_default()
                    .same_array_rebinds.insert(ordinal.0, destination.0);
                if previous.is_some_and(|previous| previous != destination.0) {
                    return Err(format!("ambiguous checked array rebind destination in {key:?}"));
                }
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
        let mut matching = facts.array_rebinds.values().filter(|(_, target, _)| target == binding);
        if !matching.all(|(point, _, destination)| {
            facts.array_calls.get(point).is_some_and(|(ordinal, _, _)|
                plan.same_array_rebinds.get(&ordinal.0) == Some(&destination.0))
        }) {
            continue;
        }
        let Some(&position) = facts.allocation_slots.get(&allocation) else { continue };
        plan.local_array_failure = Some(ClosedLocalArrayRelease {
            receipt: OriginalArrayReceipt { origin: allocation, allocation: position },
            arm: *arm,
        });
    }
    if outgoing.is_empty() {
        let mut producers = bodies.iter().filter_map(|(&key, body)| {
            if !body.is_monomorphic || key.2.is_some() {
                return None;
            }
            let facts = body.facts?;
            if !facts.checked_producer_authoritative || facts.checked_producer_backing.len() != 2 {
                return None;
            }
            let mut arms = facts.checked_producer_backing.iter();
            let ((first, left), (last, right)) = (arms.next()?, arms.next()?);
            let arms = [*facts.checked_producer_blocks.get(first)?, *facts.checked_producer_blocks.get(last)?];
            let (base, failure) = match (left.proven_allocation(), right.proven_allocation()) {
                (Some(left), Some(right)) if left == right => (left, None),
                (Some(base), None) if *right == CheckedProducerBacking::NoDescriptor => (base, Some(arms[1])),
                (None, Some(base)) if *left == CheckedProducerBacking::NoDescriptor => (base, Some(arms[0])),
                _ => return None,
            };
            let CfgPointKind::Bind { binding } = &body.cfg.points.get(base.0)?.kind else { return None };
            for &(call, ref destination, install) in facts.array_rebinds.values() {
                if destination != binding { continue }
                let (ordinal, _, _) = facts.array_calls.get(&call)?;
                let callee = target_body(&declarations, body.cfg, call)?;
                let callee_facts = bodies.get(&callee)?.facts?;
                let neutral = array_leaves.contains(&callee)
                    || array_neighbors.get(&callee).is_some_and(|dependencies| dependencies.iter().all(|leaf|
                        leaf.2.is_none() && array_leaves.contains(leaf)
                            && bodies.get(leaf).and_then(|body| body.facts).is_some_and(|facts| facts.array_neutral)));
                if callee.2.is_some() || !callee_facts.array_neutral || !neutral
                    || result.bodies.get(&key)?.same_array_rebinds.get(&ordinal.0) != Some(&install.0)
                    || !matches!(&body.cfg.points.get(call.0)?.kind, CfgPointKind::Call {
                        output: 0,
                        target: ResolvedCallTarget::ModuleCallable {
                            concrete: ConcreteCallSelection::Declaration, ..
                        }, ..
                    })
                {
                    return None;
                }
            }
            let receipt = OriginalArrayReceipt {
                origin: base,
                allocation: *facts.allocation_slots.get(&base)?,
            };
            if failure.is_some_and(|arm| {
                !body.sites.boundary.parameters.iter().all(|parameter| ordinary_value(&parameter.ty))
                    || result.bodies.get(&key).and_then(|plan| plan.local_array_failure) != Some(ClosedLocalArrayRelease { receipt, arm })
            })
            {
                return None;
            }
            Some((key, ClosedCheckedBacking { receipt, arms, failure }))
        });
        if let Some((producer, plan)) = producers.next() {
            if producers.next().is_none() {
                let end = match closed_checked_backing_consumer(
                    reservations, &module_at, &bodies, &declarations, initializers.clone(), producer,
                )? {
                    Some((consumer, CheckedBackingIncoming::Sink(install))) => Some((consumer, install, None)),
                    Some((wrapper, CheckedBackingIncoming::Wrapper(ordinal))) => match closed_checked_backing_consumer(
                        reservations, &module_at, &bodies, &declarations, initializers, wrapper,
                    )? {
                        Some((consumer, CheckedBackingIncoming::Sink(install))) if consumer != producer =>
                            Some((consumer, install, Some((wrapper, ordinal)))),
                        _ => None,
                    },
                    None => None,
                };
                if let Some((consumer, install, wrapper)) = end {
                    result.bodies.entry(producer).or_default().checked_backing_producer = Some(plan);
                    if let Some((wrapper, ordinal)) = wrapper {
                        result.bodies.entry(wrapper).or_default().checked_backing_wrapper = Some(ordinal);
                    }
                    let receiver = result.bodies.entry(consumer).or_default();
                    receiver.checked_backing_consumer = Some(install);
                    receiver.checked_backing_optional = plan.failure.is_some();
                }
            }
        }
        install_ordinary(&mut result);
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
        let mut exits = facts.owner_endpoints(OwnerSource::Formal(slot));
        let release = if let Some(&next) = forward_edges.get(&(callee, slot)) {
            if exits.next().is_some() {
                return Err(format!("aggregate owning formal has both release and transfer in {:?} at slot {slot}", body.source));
            }
            forward_predecessors.entry(next).or_default().push((callee, slot));
            None
        } else {
            let Some((endpoint, always)) = exits.next() else {
                return Err(format!("aggregate owning input has no proved formal exit in {:?} at slot {slot}", body.source));
            };
            if !always || exits.next().is_some() {
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
        for (&call, &point) in &facts.unproved_receipt_loans {
            if target_body(&declarations, body.cfg, call).is_some_and(|target| outgoing.contains(&target)) {
                return Err(format!("aggregate owning receipt has an unproved checked loan in {:?} at point {point:?}", body.source));
            }
        }
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
                if facts.owner_endpoints(OwnerSource::Call(point)).next().is_some() {
                    return Err(format!("aggregate input transfer does not cover every owning path in {:?}", body.source));
                }
                plan.calls.insert(call.0, ClosedCall::Receive);
                continue;
            }
            let mut endpoints = facts.owner_endpoints(OwnerSource::Call(point));
            let Some(((exit, block), always)) = endpoints.next() else {
                return Err(format!(
                    "aggregate owning call has no proved endpoint in {:?}",
                    body.source
                ));
            };
            if !always || endpoints.next().is_some() {
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
            || !plan.same_array_rebinds.is_empty() || plan.scalar_descriptor_assignment.is_some()
            || plan.checked_backing_wrapper.is_some()
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
    install_ordinary(&mut result);
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
        |module| (module == 0).then_some(ModuleInput { source: &program.source, items: &program.items, structs: &program.structs, calls }),
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
                Some(ModuleInput { source: &row.source, items: &row.items, structs: &row.structs, calls: sites })
            },
            |(module, item, arm), _| self.function_result(module, item, arm).map(|result| (Some(&result.aggregate), &result.cfg)),
            reservations.modules.iter().filter_map(|module| calls.modules.get(*module)?.as_ref().ok().map(|module| &module.initialization)),
        )
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passive_raw_reader_requires_a_grounded_read_and_an_independent_checked_output() {
        let text = "%%start
leaf = overload {
    *u8(*?u8, *u8) => fn(data, foreign) {
        u8 value = unsafe { core.load<u8>(data) };
        &value
    };
    *u32(*?u8, *u32) => fn(data, foreign) {
        unsafe { core.load<u8>(data); };
        foreign
    };
};
*u8(*?u8, *u8) relay = fn(data, foreign) { leaf(data, foreign) };
*u32(*?u8, *u32) borrowed_relay = fn(data, foreign) { leaf(data, foreign) };
*u8(*?u8) unknown = fn(data) { u8 value = 0; if (data == null) { value = 0; }; &value };
*u8(*?u8) consuming = fn(data) {
    u8 value = 0;
    unsafe { value = core.load<u8>(data); core.free(data); };
    &value
};
%%end";
        for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
            let source = SourceIdentity::new("project".into(), "package".into(), "main.w".into(), "r1".into());
            let parsed = crate::parse_source(source.clone(), text.into(), &[]);
            assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
            let (typed, occurrences) = typecheck_scalar_program_with_occurrences(
                derive_scalar_declarations_from_cst(parsed.result.canonical_cst()), layout,
            );
            assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
            let program = typed.program;
            let calls = resolve_program_call_sites_with_occurrences(&program, occurrences).unwrap();
            let (updates, errors) = derive_and_converge_program_references(&program, &calls);
            assert!(errors.is_empty(), "{errors:?}");
            let mut bodies = BTreeMap::new();
            let mut declarations = HashMap::new();
            for (item, result) in &updates {
                let ScalarItem::Function(function) = &program.items[*item] else { unreachable!() };
                declarations.insert((&program.source, function.name_span), (0, *item));
                for (arm, result) in std::iter::once((None, result)).chain(
                    result.arms.iter().enumerate().map(|(index, result)| (Some(index), result)),
                ) {
                    let Some(sites) = calls.function_body(*item, arm) else { continue };
                    bodies.insert((0, *item, arm), Body {
                        facts: Some(&result.aggregate),
                        cfg: &result.cfg,
                        sites,
                        source: &program.source,
                        is_monomorphic: true,
                        supports_input_result: true,
                    });
                }
            }
            let mut proven = BTreeSet::new();
            for (item, arm, expected_error) in [
                (0, Some(0), None),
                (0, Some(1), Some("unproved ordinary raw read formal")),
                (1, None, None),
                (2, None, Some("unproved ordinary raw read formal")),
                (3, None, Some("no proven load")),
                (4, None, Some("unproved ordinary raw read use")),
            ] {
                let result = prove_read_views(((0, item, arm), ReadSource::Formal(0)), &bodies, &declarations, &mut proven);
                match expected_error {
                    Some(expected) => assert!(result.unwrap_err().contains(expected)),
                    None => result.unwrap(),
                }
            }
        }
    }

    #[test]
    fn passive_carrier_consumes_a_whole_move_in_the_exact_selected_reader() {
        let text = "%%start
struct InputBuffer { *?u8 data; u64 length; }
struct OtherBuffer { *?u32 data; u64 length; }
struct AmbiguousBuffer { *?u8 data; *?u8 spare; }
*u8(InputBuffer) read = fn(input) {
    u8 out = 0;
    if (input.data != null) {
        unsafe { out = core.load<u8>(input.data); };
    };
    &out
};
*u32(OtherBuffer) consume = fn(input) {
    u32 out = 0;
    unsafe { out = core.load<u32>(input.data); core.free(input.data); };
    &out
};
*u8(AmbiguousBuffer) ambiguous = fn(input) {
    u8 out = 0;
    unsafe { out = core.load<u8>(input.data); };
    &out
};
pick = overload {
    *u8(InputBuffer) => fn(input) { read(input) };
    *u32(OtherBuffer) => fn(input) { consume(input) };
    *u8(AmbiguousBuffer) => fn(input) { ambiguous(input) };
};
%%end";
        for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
            let source = SourceIdentity::new("project".into(), "package".into(), "main.w".into(), "r1".into());
            let parsed = crate::parse_source(source.clone(), text.into(), &[]);
            assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
            let (typed, occurrences) = typecheck_scalar_program_with_occurrences(
                derive_scalar_declarations_from_cst(parsed.result.canonical_cst()), layout,
            );
            assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
            let program = typed.program;
            let calls = resolve_program_call_sites_with_occurrences(&program, occurrences).unwrap();
            let (updates, errors) = derive_and_converge_program_references(&program, &calls);
            assert!(errors.is_empty(), "{errors:?}");
            let mut bodies = BTreeMap::new();
            let mut declarations = HashMap::new();
            for (item, result) in &updates {
                let ScalarItem::Function(function) = &program.items[*item] else { unreachable!() };
                declarations.insert((&program.source, function.name_span), (0, *item));
                for (arm, result) in std::iter::once((None, result)).chain(
                    result.arms.iter().enumerate().map(|(index, result)| (Some(index), result)),
                ) {
                    let Some(sites) = calls.function_body(*item, arm) else { continue };
                    bodies.insert((0, *item, arm), Body {
                        facts: Some(&result.aggregate),
                        cfg: &result.cfg,
                        sites,
                        source: &program.source,
                        is_monomorphic: true,
                        supports_input_result: true,
                    });
                }
            }
            let selection = updates.last().unwrap().0;
            let mut proven = BTreeSet::new();
            assert_eq!(prove_read_views(((0, selection, Some(0)), ReadSource::Carrier(0)),
                &bodies, &declarations, &mut proven), Ok(()));
            assert!(prove_read_views(((0, selection, Some(1)), ReadSource::Carrier(0)),
                &bodies, &declarations, &mut proven).is_err());
            assert!(prove_read_views(((0, selection, Some(2)), ReadSource::Carrier(0)),
                &bodies, &declarations, &mut proven).unwrap_err().contains("unproved ordinary raw read formal"));
        }
    }

    #[test]
    fn checked_loan_of_received_aggregate_cannot_escape() {
        let text = r#"%%start
struct Reading { *?u8 data; u64 length; }
Reading(u64) make_reading = fn(length) {
    u8[length] backing;
    backing[0] = 19;
    *?u8 data = null;
    unsafe { data = &?backing[0]; };
    Reading packet = { .data = data; .length = length; };
    packet
};
*Reading(u64) expose = fn(length) {
    Reading packet = make_reading(length);
    *Reading view = &packet;
    view
};
%%end"#;
        let conditional = text.replace(
            "    view\n",
            "    *Reading alias = if (length == 0) { null } else { view };\n    alias\n",
        );
        assert_ne!(text, conditional);
        for text in [text, conditional.as_str()] {
            for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
                let source = SourceIdentity::new("project".into(), "package".into(), "main.w".into(), "r1".into());
                let parsed = crate::parse_source(source.clone(), text.into(), &[]);
                assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
                let prepared = crate::prepare_scalar_project(
                    vec![(derive_scalar_declarations_from_cst(parsed.result.canonical_cst()), Vec::new())],
                    vec![source],
                    crate::ScalarPreparationInputs { target_layout },
                )
                .unwrap_or_else(|error| panic!("{:?}", error.diagnostics));
                let build = crate::analyze_scalar_build(&prepared);
                assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
                let module = &prepared.project().modules[0];
                let reservations = AggregateReservations {
                    bodies: module.items.iter().enumerate().filter_map(|(item, declaration)| {
                        matches!(declaration, ScalarItem::Function(_)).then_some(((0, item, None), true))
                    }).collect(),
                    modules: vec![0],
                };
                let error = build.aggregate(&reservations).err().expect("escaped receipt loan");
                assert!(error.contains("aggregate owning receipt has an unproved checked loan"), "{error}");
            }
        }
    }

    #[test]
    fn scope_receipts_reject_disagreement_between_paths() {
        let span = ByteSpan::new(0, 8);
        let left = ReferenceBindingId {
            source: SourceIdentity::new("project".into(), "package".into(), "main.w".into(), "r1".into()),
            function_span: span,
            declaration_span: ByteSpan::new(1, 2),
            block_span: span,
            kind: ReferenceBindingKind::Declared,
        };
        let right = ReferenceBindingId { declaration_span: ByteSpan::new(3, 4), ..left.clone() };
        let a = OwnerSource::Call(CfgPointId(1));
        let b = OwnerSource::Call(CfgPointId(2));
        let mut exit = ScopeEndpoint::default();
        exit.observe(BTreeSet::from([(a, left.clone()), (b, right.clone())]));
        exit.observe(BTreeSet::from([(b, right.clone())]));
        assert_eq!(exit.receipt(a), Some(false));
        assert_eq!(exit.receipt(b), Some(true));
        exit.observe(BTreeSet::from([(b, left.clone())]));
        assert_eq!(exit.receipt(b), Some(false));

        let mut empty_first = ScopeEndpoint::default();
        empty_first.observe(BTreeSet::new());
        empty_first.observe(BTreeSet::from([(a, left)]));
        assert_eq!(empty_first.receipt(a), Some(false));
        assert_eq!(empty_first.receipt(b), None);
    }

    #[test]
    fn aggregate_value_deadlines_preserve_consumers_but_remove_dead_branch_history() {
        let mut facts = AggregateFlowFacts::default();
        let ordinary = [0, 1, 2, 3, 4, 5, 6, 7];
        assert!(matches!(facts.value_deadlines(&ordinary), std::borrow::Cow::Borrowed(_)));
        facts.opaque_container_inputs.insert(CfgPointId(4), vec![CfgPointId(0)]);
        facts.literals.insert(CfgPointId(5), vec![(OccurrenceOrdinal(0), CfgPointId(1))]);
        facts.installations.insert(CfgPointId(6), (CfgPointId(2), BlockPosition { block: BlockOrdinal(0), item: 0 }));
        let deadlines = facts.value_deadlines(&ordinary);
        assert_eq!(deadlines.as_ref(), &[4, 5, 6, 3, 4, 5, 6, 7]);

        let mut state = AggregateEdgeState::default();
        state.record_value(CfgPointId(0), Some(Value::RawFormal(0)), Some(&deadlines));
        state.record_value(CfgPointId(1), Some(Value::View(OwnerSource::Allocation(CfgPointId(1)))), Some(&deadlines));
        state.record_value(CfgPointId(2), Some(Value::Unknown), Some(&deadlines));
        state.pending_temporaries.insert(CfgPointId(0), ReferenceScopeId(0));
        let mut other = state.clone();
        state.record_value(CfgPointId(3), Some(Value::Plain(CfgPointId(3))), Some(&deadlines));
        other.record_value(CfgPointId(3), Some(Value::Unknown), Some(&deadlines));
        assert!(state == other);

        state.expire_before(CfgPointId(4), &deadlines);
        assert_eq!(state.earliest_value_use, Some(4));
        assert_eq!(state.values.keys().copied().collect::<Vec<_>>(), vec![CfgPointId(0), CfgPointId(1), CfgPointId(2)]);
        state.expire_before(CfgPointId(6), &deadlines);
        assert_eq!(state.earliest_value_use, Some(6));
        assert_eq!(state.values.keys().copied().collect::<Vec<_>>(), vec![CfgPointId(2)]);
        state.expire_before(CfgPointId(7), &deadlines);
        assert_eq!(state.earliest_value_use, None);
        assert!(state.values.is_empty());
        assert_eq!(state.pending_temporaries.get(&CfgPointId(0)), Some(&ReferenceScopeId(0)));
    }
}
