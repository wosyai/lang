use super::*;

struct Loan {
    owner: usize,
    issued: CfgPointId,
    mutable: bool,
}

struct Install {
    version: CfgPointId,
    loan: Loan,
}

#[derive(Default)]
struct Holder {
    writes: Vec<Install>,
    reads: Vec<CfgPointId>,
}

struct Fork {
    branch: CfgPointId,
    join: CfgPointId,
}

#[derive(Default)]
struct Forks {
    first: Option<Fork>,
    later: Vec<Fork>,
}

impl Forks {
    fn last(&self) -> Option<&Fork> {
        self.later.last().or(self.first.as_ref())
    }

    fn push(&mut self, fork: Fork) {
        if self.first.is_none() { self.first = Some(fork); }
        else { self.later.push(fork); }
    }

    fn get(&self, index: usize) -> Option<&Fork> {
        if index == 0 { self.first.as_ref() } else { self.later.get(index - 1) }
    }
}

#[derive(Default)]
pub(crate) struct Author {
    pending_loan: Option<Loan>,
    holders: BTreeMap<ReferenceBindingId, Holder>,
    owners: HashMap<ReferenceBindingId, usize>,
    forks: Forks,
    arm_install: bool,
    branch: Option<CfgPointId>,
    join: Option<CfgPointId>,
    root_exit: Option<CfgPointId>,
    pending_exit: Option<(CfgPointId, ReferenceScopeId)>,
}

impl Author {
    pub(super) fn emit(
        &mut self,
        id: CfgPointId,
        kind: &CfgPointKind,
        scope: ReferenceScopeId,
        types: &ReferenceBindingTypes,
    ) -> bool {
        if self.root_exit.is_some() || self.pending_exit.is_some() || (self.pending_loan.is_some()
            && !matches!(kind, CfgPointKind::Bind { .. } | CfgPointKind::Assign { .. }))
        {
            return false;
        }
        match kind {
            CfgPointKind::Branch { .. } => scope == ReferenceScopeId(0) && self.branch.replace(id).is_none(),
            CfgPointKind::Join => scope == ReferenceScopeId(0) && self.join.replace(id).is_none(),
            CfgPointKind::ScopeExit => {
                self.pending_exit = Some((id, scope));
                true
            }
            CfgPointKind::Entry | CfgPointKind::ScopeEnter
            | CfgPointKind::Evaluate(ScalarExpression::Boolean { .. } | ScalarExpression::Integer { .. }
                | ScalarExpression::Float { .. } | ScalarExpression::Char { .. }) => true,
            CfgPointKind::Bind { binding } => match types.get(binding) {
                Some(ScalarType::CheckedReference { inner, .. }) =>
                    scope == ReferenceScopeId(0) && aggregate::ordinary_value(inner),
                Some(ty) => aggregate::ordinary_value(ty) && self.pending_loan.is_none(),
                None => false,
            },
            CfgPointKind::Assign { target, .. } => target.projections.is_empty()
                && matches!(&target.place, ScalarPlace::Name { .. })
                && self.pending_loan.is_some(),
            CfgPointKind::Read { binding } => match types.get(binding) {
                Some(ScalarType::CheckedReference { .. }) => {
                    let Some(holder) = self.holders.get_mut(binding) else { return false };
                    holder.reads.push(id);
                    true
                }
                Some(ty) => aggregate::ordinary_value(ty),
                None => false,
            },
            CfgPointKind::Borrow { place, loan }
                if loan.parent.is_none()
                    && loan.origin_place == *place
                    && place.projections.is_empty()
                    && matches!(&place.place, ScalarPlace::Name { .. })
                    && types.0.get(&place.binding).is_some_and(|binding|
                        binding.local_reborrow_scope == Some(ReferenceScopeId(0))
                        && aggregate::ordinary_value(&binding.ty)) =>
            {
                let owner = match self.owners.get(&place.binding) {
                    Some(&owner) => owner,
                    None => {
                        let owner = self.owners.len();
                        self.owners.insert(place.binding.clone(), owner);
                        owner
                    }
                };
                self.pending_loan = Some(Loan {
                    owner, issued: id,
                    mutable: loan.mode == ScalarReferenceMutability::Mutable,
                });
                true
            }
            _ => false,
        }
    }

    pub(super) fn bind(
        &mut self, binding: &ReferenceBindingId, version: CfgPointId, rhs: Option<CfgPointId>,
    ) -> bool {
        let Some(loan) = self.pending_loan.take() else { return false };
        if rhs != Some(loan.issued) || loan.issued.0 + 1 != version.0 || self.holders.contains_key(binding) {
            return false;
        }
        self.holders.insert(binding.clone(), Holder {
            writes: vec![Install { version, loan }], reads: Vec::new(),
        });
        true
    }

    pub(super) fn assign(&mut self, binding: &ReferenceBindingId, version: CfgPointId, rhs: CfgPointId) -> bool {
        let Some(loan) = self.pending_loan.take() else { return false };
        let Some(holder) = self.holders.get_mut(binding) else { return false };
        if rhs != loan.issued || loan.issued.0 + 1 != version.0 {
            return false;
        }
        self.arm_install |= self.branch.is_some();
        holder.writes.push(Install { version, loan });
        true
    }

    pub(super) fn end_scope(
        &mut self, point: CfgPointId, leaving: &super::super::control::ScopeExitBindings,
    ) -> bool {
        let Some((expected, scope)) = self.pending_exit.take() else { return false };
        if expected != point {
            return false;
        }
        if scope == ReferenceScopeId(0) {
            if self.holders.keys().chain(self.owners.keys()).any(|binding| !leaving.contains(binding)) {
                return false;
            }
            self.root_exit = Some(point);
            true
        } else {
            !leaving.iter().any(|binding| self.holders.contains_key(binding) || self.owners.contains_key(binding))
        }
    }

    pub(super) fn fork(&mut self, edges: &ReferenceConditionalEdges, points: &[ReferenceFlowPoint]) -> bool {
        if edges.terminals[1] != edges.branch
            || self.branch != Some(edges.branch) || self.join != Some(edges.join)
            || self.forks.last().is_some_and(|prior| prior.join >= edges.branch || self.arm_install)
        {
            return false;
        }
        let Some([then, bypass]) = edges.entries(points) else { return false };
        if *bypass != edges.join || then.0 != edges.branch.0 + 1 || edges.terminals[0].0 + 1 != edges.join.0
            || points[edges.terminals[0].0].successors.as_slice() != [edges.join]
            || !matches!(points[then.0].kind, CfgPointKind::ScopeEnter)
            || !matches!(points[edges.terminals[0].0].kind, CfgPointKind::ScopeExit)
            || !matches!(points[edges.join.0].kind, CfgPointKind::Join)
        {
            return false;
        }
        self.forks.push(Fork { branch: edges.branch, join: edges.join });
        self.branch = None;
        self.join = None;
        true
    }

    pub(super) fn seal(self, points: &[ReferenceFlowPoint]) -> Option<Box<Sealed>> {
        if self.branch.is_some() || self.join.is_some() || (self.arm_install && !self.forks.later.is_empty()) {
            return None;
        }
        // With no arm installs, every holder version dominates all paths until
        // its next install. Later reads can be reached by taking each unit arm.
        let alternate = self.forks.first.as_ref().filter(|_| self.arm_install);
        let exit = self.root_exit?;
        if self.pending_loan.is_some() || self.pending_exit.is_some() || exit != points.last()?.id {
            return None;
        }
        let mut owners = (0..self.owners.len()).map(|_| Owner::default()).collect::<Vec<_>>();
        let mut holders = Vec::with_capacity(self.holders.len());
        for (_, holder) in self.holders {
            // Both passes visit the authored holder events, never the complete CFG.
            // An install generates its owner on its actual arm; the next
            // reachable install or the root exit retires that holder's version.
            let mut live = Vec::new();
            for &route in Route::feasible(alternate) {
                holder.leases(alternate, route, exit, |install, end, last_read| {
                    let start = CfgPointId(install.version.0 + 1);
                    if start < end {
                        owners[install.loan.owner].active[usize::from(install.loan.mutable)].push(Interval { start, end });
                    }
                    if let Some(end) = last_read.filter(|last| start < *last) {
                        live.push(Interval { start, end });
                    }
                });
            }
            if live.is_empty() {
                continue;
            }
            let index = holders.len();
            for write in &holder.writes {
                let carriers = &mut owners[write.loan.owner].carriers;
                let mask = 1 << usize::from(write.loan.mutable);
                match carriers.last_mut() {
                    Some((holder, modes)) if *holder == index => *modes |= mask,
                    _ => carriers.push((index, mask)),
                }
            }
            compact(&mut live);
            holders.push(LiveHolder { holder, live, refined: std::sync::OnceLock::new() });
        }
        if owners.iter().all(|owner| owner.carriers.is_empty()) {
            return None;
        }
        for owner in &mut owners {
            for active in &mut owner.active { compact(active); }
        }
        Some(Box::new(Sealed { exit, forks: self.forks, arm_install: self.arm_install, holders, owner_index: self.owners, owners }))
    }
}

#[derive(Clone, Copy)]
enum Route {
    Then,
    Bypass,
}

impl Fork {
    fn takes_then_at(&self, point: CfgPointId) -> bool {
        self.branch < point && point < self.join
    }
}

impl Route {
    fn feasible(fork: Option<&Fork>) -> &'static [Self] {
        if fork.is_some() { &[Self::Then, Self::Bypass] } else { &[Self::Then] }
    }

    fn reaches(self, fork: Option<&Fork>, point: CfgPointId) -> bool {
        matches!(self, Self::Then) || fork.is_none_or(|fork| !fork.takes_then_at(point))
    }
}

impl Holder {
    fn leases(
        &self, fork: Option<&Fork>, route: Route, exit: CfgPointId,
        mut visit: impl FnMut(&Install, CfgPointId, Option<CfgPointId>),
    ) {
        let mut writes = self.writes.iter().filter(|write| route.reaches(fork, write.version)).peekable();
        let mut reads = self.reads.iter().copied().filter(|&read| route.reaches(fork, read)).peekable();
        while let Some(write) = writes.next() {
            let end = writes.peek().map_or(exit, |next| next.version);
            let mut last = None;
            while let Some(read) = reads.next_if(|read| *read < end) {
                if write.version < read {
                    last = Some(read);
                }
            }
            visit(write, end, last);
        }
    }
}

#[derive(Clone, Copy)]
struct Interval {
    start: CfgPointId,
    end: CfgPointId,
}

fn compact(intervals: &mut Vec<Interval>) {
    intervals.sort_unstable_by_key(|interval| interval.start);
    let mut retained = 0;
    for at in 0..intervals.len() {
        let interval = intervals[at];
        if retained != 0 && interval.start <= intervals[retained - 1].end {
            intervals[retained - 1].end = intervals[retained - 1].end.max(interval.end);
        } else {
            intervals[retained] = interval;
            retained += 1;
        }
    }
    intervals.truncate(retained);
}

fn contains(intervals: &[Interval], point: CfgPointId) -> bool {
    let end = intervals.partition_point(|interval| interval.start <= point);
    end != 0 && point < intervals[end - 1].end
}

struct LiveHolder {
    holder: Holder,
    live: Vec<Interval>,
    refined: std::sync::OnceLock<BTreeMap<usize, CarrierRefinement>>,
}

struct ReachingLoan {
    interval: Interval,
    issue: CfgPointId,
    mutable: bool,
}

#[derive(Default)]
struct CarrierRefinement {
    then: Vec<ReachingLoan>,
    bypass: Vec<ReachingLoan>,
}

impl CarrierRefinement {
    fn at(&self, point: CfgPointId, sole_route: bool, post_join: bool, mode: AccessMode) -> Option<Option<CfgPointId>> {
        fn at_path(facts: &[ReachingLoan], point: CfgPointId, mode: AccessMode) -> Option<CfgPointId> {
            let end = facts.partition_point(|fact| fact.interval.start <= point);
            let fact = end.checked_sub(1).and_then(|index| facts.get(index))?;
            (point < fact.interval.end && mode.accepts(fact.mutable)).then_some(fact.issue)
        }
        let then = at_path(&self.then, point, mode);
        if sole_route {
            return Some(then);
        }
        let bypass = at_path(&self.bypass, point, mode);
        if post_join {
            // Independent of route, including when the incumbent deduplicates states.
            (then == bypass).then_some(then)
        } else {
            // Before the fork, a single version can have a future use on either route.
            Some(then.or(bypass))
        }
    }
}

#[derive(Default)]
struct Owner {
    active: [Vec<Interval>; 2],
    carriers: Vec<(usize, u8)>,
}

#[derive(Clone, Copy)]
pub(super) enum AccessMode {
    Shared,
    Exclusive,
}

impl AccessMode {
    fn accepts(self, mutable: bool) -> bool {
        mutable || matches!(self, Self::Exclusive)
    }
}

pub(crate) struct Sealed {
    exit: CfgPointId,
    forks: Forks,
    arm_install: bool,
    holders: Vec<LiveHolder>,
    owner_index: HashMap<ReferenceBindingId, usize>,
    owners: Vec<Owner>,
}

impl Sealed {
    pub(super) fn matches_edge_shape(&self, index: usize, point: &ReferenceFlowPoint, fork_index: &mut usize) -> bool {
        let CfgPointId(id) = point.id;
        if id != index {
            return false;
        }
        let next = CfgPointId(index + 1);
        let fork = self.forks.get(*fork_index);
        if let Some(fork) = fork.filter(|fork| fork.branch == point.id) {
            *fork_index += 1;
            matches!(&point.kind, CfgPointKind::Branch { .. })
                && point.successors.as_slice() == [next, fork.join]
        } else if matches!(&point.kind, CfgPointKind::Branch { .. }) {
            false
        } else if point.id == self.exit {
            fork.is_none() && point.successors.is_empty()
        } else {
            point.successors.as_slice() == [next]
        }
    }

    pub(super) fn conflicts(
        &self, owner: &ReferenceBindingId, access: CfgPointId, incoming_count: usize,
        points: &[ReferenceFlowPoint],
        mode: AccessMode,
    ) -> Option<Vec<SourceSpan>> {
        let alternate = self.forks.first.as_ref().filter(|_| self.arm_install);
        let post_join = alternate.is_some_and(|fork| access > fork.join);
        if incoming_count != 1 && alternate.is_some() && !post_join {
            #[cfg(test)]
            tests::observe(3);
            return None;
        }
        let Some(&owner_index) = self.owner_index.get(owner) else { return Some(Vec::new()) };
        let summary = &self.owners[owner_index];
        // Function-wide subset forgets versions. Edge generation/retirement and
        // use-before-retirement liveness are conservative independent filters.
        let mask = match mode { AccessMode::Shared => 2, AccessMode::Exclusive => 3 };
        let mut carriers = summary.carriers.iter().filter_map(|&(holder, modes)|
            (mask & modes != 0 && contains(&self.holders[holder].live, access)).then_some(holder))
            .peekable();
        let active = contains(&summary.active[1], access)
            || (matches!(mode, AccessMode::Exclusive) && contains(&summary.active[0], access));
        #[cfg(test)]
        let borrow = matches!(&points[access.0].kind, CfgPointKind::Borrow { .. });
        if !active || carriers.peek().is_none() {
            #[cfg(test)]
            tests::observe(if borrow { 5 } else { 0 });
            return Some(Vec::new());
        }
        #[cfg(test)]
        tests::observe(if borrow { 6 } else { 1 });
        let sole_route = alternate.is_none_or(|fork| fork.takes_then_at(access));
        let mut conflicts = Vec::new();
        // Carrier indices were assigned by the authored BTreeMap in binding order.
        for holder in carriers {
            let entry = &self.holders[holder];
            let precise = entry.refined.get_or_init(|| {
                #[cfg(test)]
                tests::observe(2);
                let mut by_owner = BTreeMap::<usize, CarrierRefinement>::new();
                for &route in Route::feasible(alternate) {
                    entry.holder.leases(alternate, route, self.exit, |install, _, last_read| {
                        let start = CfgPointId(install.version.0 + 1);
                        let Some(end) = last_read.filter(|&last| start < last) else { return };
                        let carrier = by_owner.entry(install.loan.owner).or_default();
                        let facts = match route {
                            Route::Then => &mut carrier.then,
                            Route::Bypass => &mut carrier.bypass,
                        };
                        facts.push(ReachingLoan {
                            interval: Interval { start, end }, issue: install.loan.issued, mutable: install.loan.mutable,
                        });
                    });
                }
                by_owner
            });
            let Some(carrier) = precise.get(&owner_index) else { continue };
            let Some(issue) = carrier.at(access, sole_route, post_join, mode) else {
                #[cfg(test)]
                tests::observe(3);
                return None;
            };
            let Some(issue) = issue else { continue };
            let CfgPointKind::Borrow { place, loan } = &points.get(issue.0)?.kind else {
                return None;
            };
            if &place.binding != owner || loan.origin_place != *place
                || !place.projections.is_empty() || !matches!(&place.place, ScalarPlace::Name { .. })
                || loan.parent.is_some() || !mode.accepts(loan.mode == ScalarReferenceMutability::Mutable)
            {
                return None;
            }
            conflicts.push(SourceSpan::new(loan.origin_place.source.clone(), loan.creation_span));
        }
        Some(conflicts)
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    thread_local! {
        static COUNTS: std::cell::Cell<[usize; 7]> = const { std::cell::Cell::new([0; 7]) };
    }

    pub(in crate::scalar::reference) fn observe(stage: usize) {
        COUNTS.with(|counts| {
            let mut next = counts.get();
            next[stage] += 1;
            counts.set(next);
        });
    }

    fn prepared() -> FunctionCallSites {
        let span = ByteSpan::new(0, 100);
        FunctionCallSites {
            boundary: CallableBoundary { parameters: vec![], results: vec![], root_outputs: vec![], generics: vec![] },
            sites: vec![], arms: vec![], occurrences: vec![], call_occurrences: vec![],
            declared_unsafe_calls: BTreeSet::new(), callable_escapes: HashSet::new(),
            overload_reservation_sites: vec![], self_tail_candidates: vec![], allocation_length_calls: vec![],
            allocation_length_call_targets: HashMap::new(), checked_call_signatures: HashMap::new(),
            plain_names: PlainNameFacts::Prepared(HashMap::new()), named_place_roots: NamedPlaceRootFacts::Prepared(HashMap::new()),
            direct_destinations: DirectDestinationFacts::Prepared(HashMap::new()), members: MemberFacts::Prepared(HashMap::new()),
            query_allocation_outputs: HashMap::new(), structure: OccurrenceStructure::new(span),
            assignments: HashMap::new(), short_circuits: HashMap::new(),
            conditionals: HashMap::from([(OccurrenceOrdinal(0), ConditionalRegion::Unit {
                span, condition: OccurrenceOrdinal(1), then_block: BlockOrdinal(1), fallthrough: OccurrenceOrdinal(2),
            })]),
        }
    }

    fn function(span: ByteSpan) -> ScalarFunction {
        ScalarFunction {
            name: "inert".into(), name_span: span,
            signature: ScalarType::Callable { parameters: vec![], outputs: ScalarOutputSequence { outputs: vec![], span } },
            parameters: vec![], parameter_spans: vec![],
            body: ScalarBlock { items: vec![], terminated_items: vec![], expressions: vec![], final_output_values: vec![], span, unsafe_context: false },
            span, generic_parameters: vec![], overload_arms: vec![],
            reference_cfg: ScalarReferenceCfg {
                facts: vec![], points: vec![], initial_points: vec![], allocations: BTreeSet::new(),
                specialized_policies: vec![], primary_call_points: HashMap::new(),
            },
        }
    }

    fn loan(builder: &mut ReferenceFlowBuilder<'_>, owner: &ReferenceBindingId, span: ByteSpan)
        -> (CfgPointId, BTreeSet<ReferenceOriginId>)
    {
        loan_with_mode(builder, owner, span, ScalarReferenceMutability::Mutable)
    }

    fn loan_with_mode(
        builder: &mut ReferenceFlowBuilder<'_>, owner: &ReferenceBindingId, span: ByteSpan,
        mode: ScalarReferenceMutability,
    ) -> (CfgPointId, BTreeSet<ReferenceOriginId>) {
        let place = builder.reference_place(&ScalarPlace::Name { name: "owner".into(), span }, owner.clone(), None);
        let loan = ReferenceLoanId {
            origin_place: place.clone(), creation_span: span, parent: None, mode,
        };
        let id = builder.intern(ReferenceOrigin::Fresh {
            allocation: ReturnedAllocationId {
                source: builder.source.clone(), function_span: builder.function_span,
                declaration_span: owner.declaration_span, creation_span: span,
            },
            loan: loan.clone(),
        });
        let origins = BTreeSet::from([id]);
        let point = builder.point(CfgPointKind::Borrow { place, loan }, span, ReferenceScopeId(0), origins.clone());
        (point, origins)
    }

    fn bind(
        builder: &mut ReferenceFlowBuilder<'_>, binding: &ReferenceBindingId, item: usize,
        initializer: CfgPointId, origins: BTreeSet<ReferenceOriginId>,
    ) {
        builder.checked.insert(binding.clone());
        builder.origins.insert(binding.clone(), origins.clone());
        builder.point(CfgPointKind::Bind { binding: binding.clone() }, binding.declaration_span, ReferenceScopeId(0), origins);
        builder.record_binding_definition(BlockPosition { block: BlockOrdinal(0), item }, 0, false, Some(initializer));
    }

    fn inert_result(prepared_existing: bool, divergent: bool, borrow_matrix: bool, linear: bool, sequential: bool) -> FunctionReferenceResult {
        let source = SourceIdentity::new("project".into(), "package".into(), "metadata".into(), "r1".into());
        let at = |start| ByteSpan::new(start, start + 1);
        let span = ByteSpan::new(0, 100);
        let function = function(span);
        let mut calls = prepared();
        let binding = |start| ReferenceBindingId {
            source: source.clone(), function_span: span, block_span: span,
            declaration_span: at(start), kind: ReferenceBindingKind::Declared,
        };
        let (x, y, h1, h2, h3, z, h4) = (binding(20), binding(21), binding(22), binding(23), binding(24), binding(25), binding(26));
        if linear { calls.conditionals.clear(); }
        if sequential {
            calls.conditionals.insert(OccurrenceOrdinal(8), ConditionalRegion::Unit {
                span, condition: OccurrenceOrdinal(9), then_block: BlockOrdinal(2),
                fallthrough: OccurrenceOrdinal(10),
            });
        }
        if linear || sequential {
            let NamedPlaceRootFacts::Prepared(roots) = &mut calls.named_place_roots else { unreachable!() };
            for (occurrence, owner) in [(1, &x), (3, &y), (5, &x), (15, &y), (34, &z), (76, &z)] {
                roots.insert(OccurrenceOrdinal(occurrence), NamedPlaceRootIdentity::Lexical(owner.clone()));
            }
            if borrow_matrix {
                for occurrence in 40..=44 {
                    roots.insert(OccurrenceOrdinal(occurrence), NamedPlaceRootIdentity::Lexical(z.clone()));
                }
            }
        }
        let names = HashSet::new();
        let mut builder = ReferenceFlowBuilder::new(&source, &function, &names, &calls, &[], &[]);
        assert!(builder.local_borrow.is_some());
        let root = ReferenceScopeId(0);
        let arm = ReferenceScopeId(1);
        for owner in [&x, &y, &z] {
            builder.binding_types.record(owner.clone(), ScalarType::I32, Some(root));
        }
        for holder in [&h1, &h2, &h3, &h4] {
            builder.binding_types.record(holder.clone(), ScalarType::CheckedReference {
                inner: Box::new(ScalarType::I32), mutability: ScalarReferenceMutability::Mutable,
            }, Some(root));
        }
        builder.point(CfgPointKind::Entry, span, root, BTreeSet::new());
        builder.point(CfgPointKind::ScopeEnter, span, root, BTreeSet::new());
        let (initial, origins) = loan(&mut builder, &x, at(1));
        bind(&mut builder, &h2, 0, initial, origins);
        builder.point(CfgPointKind::Read { binding: y.clone() }, at(2), root, BTreeSet::new());
        let (third, third_origins) = loan(&mut builder, &y, at(15));
        bind(&mut builder, &h3, 1, third, third_origins.clone());
        let (rhs, origins) = loan(&mut builder, &y, at(3));
        let target = builder.reference_place(&ScalarPlace::Name { name: "h2".into(), span: at(4) }, h2.clone(), None);
        let emitted = builder.emit_direct_assignment(h2.clone(), CfgPointKind::Assign {
            target, rhs_point: rhs, previous_origins: BTreeSet::new(), previous_loans: vec![],
        }, rhs, ReferenceValue::Checked(origins.clone()), at(4), root).expect("typed direct assignment");
        builder.author_direct_assignment(emitted, prepared_existing);
        let (second, second_origins) = loan(&mut builder, &x, at(5));
        bind(&mut builder, &h1, 2, second, second_origins);
        let mut extra_exit = Vec::new();
        if borrow_matrix {
            let mut old = Vec::new();
            for (item, (start, owner, mode)) in [
                (40, &z, ScalarReferenceMutability::Shared),
                (41, &z, ScalarReferenceMutability::Shared),
                (42, &z, ScalarReferenceMutability::Mutable),
                (43, &z, ScalarReferenceMutability::Shared),
            ].into_iter().enumerate() {
                let holder = binding(60 + item as u32);
                builder.binding_types.record(holder.clone(), ScalarType::CheckedReference {
                    inner: Box::new(ScalarType::I32), mutability: mode,
                }, Some(root));
                let (issue, origins) = loan_with_mode(&mut builder, owner, at(start), mode);
                bind(&mut builder, &holder, 10 + item, issue, origins.clone());
                old.push((holder.clone(), origins));
                extra_exit.push(holder);
            }
            let (retired, previous) = &old[2];
            builder.point(CfgPointKind::Read { binding: retired.clone() }, at(47), root, previous.clone());
            let (rhs, origins) = loan(&mut builder, &z, at(44));
            let target = builder.reference_place(&ScalarPlace::Name { name: "retired".into(), span: at(45) }, retired.clone(), None);
            let emitted = builder.emit_direct_assignment(retired.clone(), CfgPointKind::Assign {
                target, rhs_point: rhs, previous_origins: BTreeSet::new(), previous_loans: vec![],
            }, rhs, ReferenceValue::Checked(origins.clone()), at(45), root).expect("typed replacing assignment");
            builder.author_direct_assignment(emitted, prepared_existing);
            old[2].1 = origins;
            for (holder, origins) in old {
                builder.point(CfgPointKind::Read { binding: holder }, at(46), root, origins);
            }
        }
        let (unread, unread_origins) = loan(&mut builder, &z, at(34));
        bind(&mut builder, &h4, 3, unread, unread_origins);
        let divergent_origins;
        if linear {
            assert!(!divergent);
            divergent_origins = None;
            builder.point(CfgPointKind::Read { binding: x.clone() }, at(8), root, BTreeSet::new());
            builder.point(CfgPointKind::Read { binding: y.clone() }, at(9), root, BTreeSet::new());
        } else {
            let branch = builder.point(CfgPointKind::Branch { condition: ScalarExpression::Boolean { value: true, span: at(6) } }, at(6), root, BTreeSet::new());
            builder.point(CfgPointKind::ScopeEnter, at(7), arm, BTreeSet::new());
            builder.point(CfgPointKind::Read { binding: x.clone() }, at(8), arm, BTreeSet::new());
            for index in [9, 10] {
                builder.point(CfgPointKind::Read { binding: y.clone() }, at(index), arm, BTreeSet::new());
            }
            divergent_origins = if divergent {
                let (rhs, origins) = loan(&mut builder, &x, at(30));
                let target = builder.reference_place(
                    &ScalarPlace::Name { name: "h1".into(), span: at(31) }, h1.clone(), None,
                );
                let emitted = builder.emit_direct_assignment(h1.clone(), CfgPointKind::Assign {
                    target, rhs_point: rhs, previous_origins: BTreeSet::new(), previous_loans: vec![],
                }, rhs, ReferenceValue::Checked(origins.clone()), at(31), arm).expect("typed branch assignment");
                builder.author_direct_assignment(emitted, prepared_existing);
                Some(origins)
            } else { None };
            let then_end = builder.point(CfgPointKind::ScopeExit, at(11), arm, BTreeSet::new());
            builder.author_local_scope_exit(then_end, &super::super::super::control::ScopeExitBindings::default(), true);
            builder.cursor = None;
            let join = builder.point(CfgPointKind::Join, at(12), root, BTreeSet::new());
            builder.connect_reference_conditional(
                &ReferenceConditionalEdges { branch, terminals: [then_end, branch], join }, &calls.conditionals[&OccurrenceOrdinal(0)],
            );
        }
        builder.point(CfgPointKind::Read { binding: z.clone() }, at(35), root, BTreeSet::new());
        builder.point(CfgPointKind::Read { binding: y.clone() }, at(13), root, BTreeSet::new());
        if let Some(origins) = divergent_origins {
            builder.point(CfgPointKind::Read { binding: x.clone() }, at(32), root, BTreeSet::new());
            builder.point(CfgPointKind::Read { binding: h1.clone() }, at(33), root, origins);
        }
        if sequential {
            let between = if divergent { None } else {
                let (rhs, origins) = loan(&mut builder, &z, at(76));
                let target = builder.reference_place(&ScalarPlace::Name { name: "h1".into(), span: at(77) }, h1.clone(), None);
                let emitted = builder.emit_direct_assignment(h1.clone(), CfgPointKind::Assign {
                    target, rhs_point: rhs, previous_origins: BTreeSet::new(), previous_loans: vec![],
                }, rhs, ReferenceValue::Checked(origins.clone()), at(77), root).expect("typed inter-fork assignment");
                builder.author_direct_assignment(emitted, prepared_existing);
                Some(origins)
            };
            let second_arm = ReferenceScopeId(2);
            let branch = builder.point(CfgPointKind::Branch {
                condition: ScalarExpression::Boolean { value: true, span: at(70) },
            }, at(70), root, BTreeSet::new());
            builder.point(CfgPointKind::ScopeEnter, at(71), second_arm, BTreeSet::new());
            let local = ReferenceBindingId { block_span: ByteSpan::new(71, 73), ..binding(79) };
            builder.binding_types.record(local.clone(), ScalarType::I32, Some(second_arm));
            builder.point(CfgPointKind::Bind { binding: local.clone() }, at(79), second_arm, BTreeSet::new());
            builder.point(CfgPointKind::Read { binding: h2.clone() }, at(14), second_arm, origins);
            builder.point(CfgPointKind::Read { binding: h3.clone() }, at(16), second_arm, third_origins);
            if let Some(origins) = between {
                builder.point(CfgPointKind::Read { binding: h1.clone() }, at(78), second_arm, origins);
            }
            let then_end = builder.point(CfgPointKind::ScopeExit, at(72), second_arm, BTreeSet::new());
            let mut arm_leaving = super::super::super::control::ScopeExitBindings::default();
            arm_leaving.insert(local);
            builder.author_local_scope_exit(then_end, &arm_leaving, true);
            builder.proof_points.scope_exit_bindings.as_mut().unwrap().insert(then_end, arm_leaving);
            builder.cursor = None;
            let join = builder.point(CfgPointKind::Join, at(73), root, BTreeSet::new());
            builder.connect_reference_conditional(
                &ReferenceConditionalEdges { branch, terminals: [then_end, branch], join },
                &calls.conditionals[&OccurrenceOrdinal(8)],
            );
        } else {
            builder.point(CfgPointKind::Read { binding: h2.clone() }, at(14), root, origins);
            builder.point(CfgPointKind::Read { binding: h3.clone() }, at(16), root, third_origins);
        }
        let exit = builder.point(CfgPointKind::ScopeExit, span, root, BTreeSet::new());
        let mut leaving = super::super::super::control::ScopeExitBindings::default();
        for binding in [x, y, h1, h2, h3, z, h4] {
            leaving.insert(binding);
        }
        for binding in extra_exit {
            leaving.insert(binding);
        }
        builder.author_local_scope_exit(exit, &leaving, true);
        builder.proof_points.scope_exit_bindings.as_mut().unwrap().insert(exit, leaving);
        builder.seal_local_borrow();
        builder.cfg.initial_points = builder.cfg.points.clone();
        FunctionReferenceResult {
            cfg: builder.cfg, proof_points: builder.proof_points,
            checked_contract_returns: vec![], aggregate: aggregate::AggregateFlowResult::default(), arms: vec![],
        }
    }

    fn definitive(result: &mut FunctionReferenceResult) -> Vec<ReferenceAnalysisError> {
        let contracts = ReferenceOutputContracts::default();
        let mut replays = Vec::new();
        transfer_function_output_contracts(
            result, &contracts, &contracts, &mut HashMap::new(), &mut replays, &mut 0,
        );
        replays.pop().expect("definitive replay").errors
    }

    #[test]
    fn production_builder_and_pristine_replay_replace_only_complete_local_read_verdicts() {
        let mut admitted = inert_result(true, false, false, false, false);
        assert!(admitted.proof_points.local_borrow.is_some());
        COUNTS.with(|counts| counts.set([0; 7]));
        let staged = definitive(&mut admitted);
        let counts = COUNTS.with(|counts| counts.get());
        assert_eq!(&counts[..5], &[2, 4, 2, 0, 0]);
        let again = definitive(&mut admitted);
        assert_eq!(staged, again);
        assert_eq!(COUNTS.with(|counts| counts.get()[2]), 2);
        let mut unprepared = inert_result(false, false, false, false, false);
        assert!(unprepared.proof_points.local_borrow.is_none());
        COUNTS.with(|counts| counts.set([0; 7]));
        let incumbent = definitive(&mut unprepared);
        assert_eq!(staged, incumbent);
        assert_eq!(COUNTS.with(|counts| counts.get()), [0; 7]);
        let spans = staged.iter().find_map(|error| match error {
            ReferenceAnalysisError::ConflictingPlace { source_span, origins }
                if source_span.range == ByteSpan::new(9, 10) => Some(origins),
            _ => None,
        }).expect("two ordered holders");
        assert_eq!(spans.iter().map(|origin| origin.range).collect::<Vec<_>>(),
            vec![ByteSpan::new(3, 4), ByteSpan::new(15, 16)]);
        assert!(!staged.iter().any(|error| matches!(error,
            ReferenceAnalysisError::ConflictingPlace { source_span, .. } if source_span.range == ByteSpan::new(8, 9))));
    }
    #[test]
    fn production_post_join_agreement_preserves_state_multiplicity_and_divergence_falls_back() {
        fn at(errors: &[ReferenceAnalysisError], start: u32) -> Vec<Vec<ByteSpan>> {
            errors.iter().filter_map(|error| match error {
                ReferenceAnalysisError::ConflictingPlace { source_span, origins }
                    if source_span.range == ByteSpan::new(start, start + 1) =>
                    Some(origins.iter().map(|origin| origin.range).collect()),
                _ => None,
            }).collect()
        }
        for divergent in [false, true] {
            let mut admitted = inert_result(true, divergent, false, false, false);
            assert!(admitted.proof_points.local_borrow.is_some());
            COUNTS.with(|counts| counts.set([0; 7]));
            let staged = definitive(&mut admitted);
            let counts = COUNTS.with(|counts| counts.get());
            let mut incumbent = inert_result(false, divergent, false, false, false);
            assert_eq!(staged, definitive(&mut incumbent));
            assert_eq!(at(&staged, 13), vec![vec![ByteSpan::new(3, 4), ByteSpan::new(15, 16)]]);
            assert_eq!(counts[4], if divergent { 2 } else { 0 });
            assert_eq!(counts[1], if divergent { 5 } else { 4 });
            assert_eq!(counts[0], 2);
            assert!(at(&staged, 35).is_empty());
            assert_eq!(counts[3], usize::from(divergent));
            if divergent {
                let mut divergent_origins = at(&staged, 32);
                divergent_origins.sort_unstable_by_key(|spans| spans[0].start);
                assert_eq!(divergent_origins, vec![vec![ByteSpan::new(5, 6)], vec![ByteSpan::new(30, 31)]]);
            }
        }
    }

    #[test]
    fn production_borrow_modes_use_only_conflicting_live_incoming_loans() {
        let mut admitted = inert_result(true, false, true, false, false);
        let mut incumbent = inert_result(false, false, true, false, false);
        assert!(admitted.proof_points.local_borrow.is_some());
        COUNTS.with(|counts| counts.set([0; 7]));
        let errors = definitive(&mut admitted);
        let counts = COUNTS.with(|counts| counts.get());
        assert_eq!(errors, definitive(&mut incumbent));
        assert!(counts[5] > 0 && counts[6] > 0);
        let spans_at = |start| errors.iter().find_map(|error| match error {
            ReferenceAnalysisError::ConflictingPlace { source_span, origins }
                if source_span.range == ByteSpan::new(start, start + 1) =>
                Some(origins.iter().map(|origin| origin.range).collect::<Vec<_>>()),
            _ => None,
        });
        assert_eq!(spans_at(41), None);
        assert_eq!(spans_at(42), Some(vec![ByteSpan::new(40, 41), ByteSpan::new(41, 42)]));
        assert_eq!(spans_at(43), Some(vec![ByteSpan::new(42, 43)]));
        // Its earlier read is irrelevant once the next install retires that version.
        assert_eq!(spans_at(44), Some(vec![ByteSpan::new(40, 41), ByteSpan::new(41, 42), ByteSpan::new(43, 44)]));
    }

    #[test]
    fn production_straight_line_flow_seals_one_route_and_keeps_reference_diagnostics() {
        let mut admitted = inert_result(true, false, true, true, false);
        let mut incumbent = inert_result(false, false, true, true, false);
        let sealed = admitted.proof_points.local_borrow.as_ref().expect("complete linear proof");
        assert!(sealed.forks.first.is_none());
        assert!(incumbent.proof_points.local_borrow.is_none());
        COUNTS.with(|counts| counts.set([0; 7]));
        let staged = definitive(&mut admitted);
        let counts = COUNTS.with(|counts| counts.get());
        assert_eq!(staged, definitive(&mut incumbent));
        assert_eq!(&counts[..2], &[2, 3]);
        assert_eq!(counts[3], 0);
        assert!(counts[5] > 0 && counts[6] > 0);
        assert!(staged.iter().any(|error| matches!(error,
            ReferenceAnalysisError::ConflictingPlace { source_span, .. }
                if source_span.range == ByteSpan::new(9, 10))));
        assert!(!staged.iter().any(|error| matches!(error,
            ReferenceAnalysisError::ConflictingPlace { source_span, .. }
                if source_span.range == ByteSpan::new(8, 9))));
    }

    #[test]
    fn production_straight_line_without_named_places_skips_authoring() {
        let source = SourceIdentity::new("project".into(), "package".into(), "metadata".into(), "r1".into());
        let span = ByteSpan::new(0, 100);
        let function = function(span);
        let mut calls = prepared();
        calls.conditionals.clear();
        let names = HashSet::new();
        let mut builder = ReferenceFlowBuilder::new(&source, &function, &names, &calls, &[], &[]);
        assert!(builder.local_borrow.is_none());
        builder.point(CfgPointKind::Entry, span, ReferenceScopeId(0), BTreeSet::new());
        builder.point(CfgPointKind::ScopeEnter, span, ReferenceScopeId(0), BTreeSet::new());
        let exit = builder.point(CfgPointKind::ScopeExit, span, ReferenceScopeId(0), BTreeSet::new());
        builder.author_local_scope_exit(exit, &super::super::super::control::ScopeExitBindings::default(), true);
        builder.seal_local_borrow();
        assert!(builder.proof_points.local_borrow.is_none());
    }

    #[test]
    fn production_sequential_unit_arms_share_dominating_holder_versions() {
        let mut admitted = inert_result(true, false, false, false, true);
        let mut incumbent = inert_result(false, false, false, false, true);
        let mut branch_store = inert_result(true, true, false, false, true);
        let mut branch_store_incumbent = inert_result(false, true, false, false, true);
        let sealed = admitted.proof_points.local_borrow.as_ref().expect("complete fork sequence");
        assert_eq!(sealed.forks.later.len(), 1);
        assert!(!sealed.arm_install);
        assert_eq!(Route::feasible(sealed.forks.first.as_ref().filter(|_| sealed.arm_install)).len(), 1);
        assert!(incumbent.proof_points.local_borrow.is_none());
        assert!(branch_store.proof_points.local_borrow.is_none());
        COUNTS.with(|counts| counts.set([0; 7]));
        let staged = definitive(&mut admitted);
        let counts = COUNTS.with(|counts| counts.get());
        assert_eq!(staged, definitive(&mut incumbent));
        assert_eq!(definitive(&mut branch_store), definitive(&mut branch_store_incumbent));
        assert_eq!(counts[3], 0);
        assert!(counts[0] > 0 && counts[1] > 0);
        assert!(staged.iter().any(|error| matches!(error,
            ReferenceAnalysisError::ConflictingPlace { source_span, origins }
                if source_span.range == ByteSpan::new(9, 10) && origins.len() == 2)));
        assert!(!staged.iter().any(|error| matches!(error,
            ReferenceAnalysisError::ConflictingPlace { source_span, .. }
                if source_span.range == ByteSpan::new(8, 9))));
    }

}
