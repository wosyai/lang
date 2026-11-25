use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
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
    arms: Option<Box<ArmRuns>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Arm(usize);

impl Arm {
    fn new(fork: usize, second: bool) -> Self { Self((fork << 1) | usize::from(second)) }
    fn fork(self) -> usize { self.0 >> 1 }
    fn second(self) -> bool { self.0 & 1 != 0 }
}

struct ArmRun {
    start: usize,
    end: usize,
    arm: Arm,
}

#[derive(Default)]
struct ArmRuns {
    reads: Vec<ArmRun>,
    writes: Vec<ArmRun>,
    second_read: bool,
}

fn record_arm(runs: &mut Vec<ArmRun>, index: usize, arm: Arm) {
    if let Some(run) = runs.last_mut().filter(|run| run.end == index && run.arm == arm) {
        run.end += 1;
    } else {
        runs.push(ArmRun { start: index, end: index + 1, arm });
    }
}

fn next_arm(runs: &mut &[ArmRun], index: usize) -> Option<Arm> {
    while let Some((run, remaining)) = runs.split_first() {
        if index < run.end { return (index >= run.start).then_some(run.arm); }
        *runs = remaining;
    }
    None
}

struct Fork {
    branch: CfgPointId,
    join: CfgPointId,
    second: Option<CfgPointId>,
}

#[derive(Default)]
struct Forks {
    first: Option<Fork>,
    later: Vec<Fork>,
}

impl Forks {
    fn len(&self) -> usize { usize::from(self.first.is_some()) + self.later.len() }

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
    branch: Option<CfgPointId>,
    second: Option<CfgPointId>,
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
                && (self.pending_loan.is_some()
                    || types.get(&target.binding).is_some_and(aggregate::ordinary_value)),
            CfgPointKind::Read { binding } => match types.get(binding) {
                Some(ScalarType::CheckedReference { .. }) => {
                    let arm = self.arm();
                    let Some(holder) = self.holders.get_mut(binding) else { return false };
                    holder.record_read(id, arm);
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
            writes: vec![Install { version, loan }], ..Holder::default()
        });
        true
    }

    pub(super) fn assign(&mut self, binding: &ReferenceBindingId, version: CfgPointId, rhs: CfgPointId) -> bool {
        let Some(loan) = self.pending_loan.take() else { return false };
        let arm = self.arm();
        let Some(holder) = self.holders.get_mut(binding) else { return false };
        if rhs != loan.issued || loan.issued.0 + 1 != version.0 {
            return false;
        }
        holder.record_install(Install { version, loan }, arm);
        true
    }

    pub(super) fn accept_ordinary_store(&self) -> bool {
        self.pending_loan.is_none()
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

    fn arm(&self) -> Option<Arm> {
        self.branch.map(|_| Arm::new(self.forks.len(), self.second.is_some()))
    }

    pub(super) fn second_arm(&mut self, entry: CfgPointId) -> bool {
        self.branch.is_some() && self.join.is_none() && self.pending_loan.is_none()
            && self.pending_exit.is_none() && self.second.replace(entry).is_none()
    }

    pub(super) fn fork(
        &mut self, edges: &ReferenceConditionalEdges, points: &[ReferenceFlowPoint], explicit: bool,
    ) -> bool {
        if explicit != self.second.is_some()
            || self.branch != Some(edges.branch) || self.join != Some(edges.join)
            || self.forks.last().is_some_and(|prior| prior.join >= edges.branch)
        {
            return false;
        }
        let Some([then, alternative]) = edges.entries(points) else { return false };
        let arm = |entry: CfgPointId, end: CfgPointId, next: CfgPointId| {
            entry < end && end.0 + 1 == next.0
                && matches!(points[entry.0].kind, CfgPointKind::ScopeEnter)
                && matches!(points[end.0].kind, CfgPointKind::ScopeExit)
                && points[end.0].successors.as_slice() == [edges.join]
        };
        if then.0 != edges.branch.0 + 1
            || !arm(*then, edges.terminals[0], *alternative)
            || !matches!(points[edges.join.0].kind, CfgPointKind::Join)
            || match self.second {
                Some(entry) => entry != *alternative || !arm(entry, edges.terminals[1], edges.join),
                None => *alternative != edges.join || edges.terminals[1] != edges.branch,
            }
        {
            return false;
        }
        self.forks.push(Fork { branch: edges.branch, join: edges.join, second: self.second.take() });
        self.branch = None;
        self.join = None;
        true
    }

    pub(super) fn seal(self, points: &[ReferenceFlowPoint]) -> Option<Box<Sealed>> {
        if self.branch.is_some() || self.join.is_some() || self.second.is_some() {
            return None;
        }
        let exit = self.root_exit?;
        if self.pending_loan.is_some() || self.pending_exit.is_some() || exit != points.last()?.id {
            return None;
        }
        let mut owners = (0..self.owners.len()).map(|_| Owner::default()).collect::<Vec<_>>();
        let mut holders = Vec::with_capacity(self.holders.len());
        for (_, holder) in self.holders {
            let Some(&last_read) = holder.reads.last() else { continue };
            let mut live = Vec::new();
            let (_, mut write_arms) = holder.arm_runs();
            if !write_arms.is_empty() {
                let mut retirement = exit;
                for (index, install) in holder.writes.iter().enumerate().rev() {
                    while write_arms.last().is_some_and(|run| run.start > index) {
                        write_arms = &write_arms[..write_arms.len() - 1];
                    }
                    let start = CfgPointId(install.version.0 + 1);
                    if start < retirement {
                        owners[install.loan.owner].active[usize::from(install.loan.mutable)]
                            .push(Interval { start, end: retirement });
                    }
                    if write_arms.last().is_none_or(|run| !(run.start..run.end).contains(&index)) {
                        retirement = install.version;
                    }
                }
                let start = CfgPointId(holder.writes[0].version.0 + 1);
                if start < last_read { live.push(Interval { start, end: last_read }); }
            } else {
                holder.live_linear_leases(exit, |loan, interval| {
                    owners[loan.owner].active[usize::from(loan.mutable)].push(interval);
                    live.push(interval);
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
        Some(Box::new(Sealed { exit, forks: self.forks, holders, owner_index: self.owners, owners }))
    }
}

impl Holder {
    fn record_read(&mut self, point: CfgPointId, arm: Option<Arm>) {
        if let Some(fork) = arm {
            let arms = self.arms.get_or_insert_with(Box::default);
            arms.second_read |= fork.second();
            record_arm(&mut arms.reads, self.reads.len(), fork);
        }
        self.reads.push(point);
    }

    fn record_install(&mut self, install: Install, arm: Option<Arm>) {
        if let Some(fork) = arm {
            record_arm(&mut self.arms.get_or_insert_with(Box::default).writes, self.writes.len(), fork);
        }
        self.writes.push(install);
    }

    fn arm_runs(&self) -> (&[ArmRun], &[ArmRun]) {
        match self.arms.as_deref() {
            Some(arms) => (&arms.reads, &arms.writes),
            None => (&[], &[]),
        }
    }

    fn live_linear_leases(&self, exit: CfgPointId, mut visit: impl FnMut(Loan, Interval)) {
        let mut writes = self.writes.iter().peekable();
        let mut reads = self.reads.iter().copied().peekable();
        while reads.peek().is_some() {
            let Some(write) = writes.next() else { break };
            let retire = writes.peek().map_or(exit, |next| next.version);
            let start = CfgPointId(write.version.0 + 1);
            let mut last = None;
            while let Some(read) = reads.next_if(|read| *read < retire) {
                if start < read { last = Some(read); }
            }
            if let Some(end) = last {
                visit(write.loan, Interval { start, end });
            }
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
    refined: std::sync::OnceLock<Option<Refinement>>,
}

struct LinearLease {
    interval: Interval,
    loan: Loan,
}

enum Refinement {
    Linear(Vec<LinearLease>),
    Branching(Vec<Boundary>),
}

impl Refinement {
    fn facts_at(&self, point: CfgPointId) -> (Reaching, bool) {
        match self {
            Self::Linear(facts) => {
                let end = facts.partition_point(|fact| fact.interval.start <= point);
                match end.checked_sub(1).and_then(|index| facts.get(index)) {
                    Some(fact) if point < fact.interval.end => (Reaching::One(fact.loan), true),
                    _ => (Reaching::Absent, false),
                }
            }
            Self::Branching(events) => access_facts(events, point),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Reaching {
    Absent,
    One(Loan),
    Diverse,
}

#[derive(Clone, Copy)]
enum Event {
    Read,
    Install(Loan),
    Branch,
    ThenEnd,
    SecondEntry,
    Join,
}

struct Boundary {
    point: CfgPointId,
    event: Event,
    reaching_before: Reaching,
    reaching_after: Reaching,
    live_before: bool,
    live_after: bool,
}

impl Boundary {
    fn new(point: CfgPointId, event: Event) -> Self {
        Self {
            point, event, reaching_before: Reaching::Absent, reaching_after: Reaching::Absent,
            live_before: false, live_after: false,
        }
    }
}

impl Holder {
    fn refine(&self, forks: &Forks, exit: CfgPointId) -> Option<Refinement> {
        if self.arms.as_deref().is_none_or(|arms| arms.writes.is_empty() && !arms.second_read) {
            let mut leases = Vec::new();
            self.live_linear_leases(exit, |loan, interval| leases.push(LinearLease { interval, loan }));
            return Some(Refinement::Linear(leases));
        }
        self.branch_boundaries(forks).map(Refinement::Branching)
    }

    fn branch_boundaries(&self, forks: &Forks) -> Option<Vec<Boundary>> {
        let mut writes = self.writes.iter().enumerate().peekable();
        let mut reads = self.reads.iter().copied().enumerate().peekable();
        let (mut read_arms, mut write_arms) = self.arm_runs();
        let mut events = Vec::new();
        let mut current_arm: Option<Arm> = None;
        while writes.peek().is_some() || reads.peek().is_some() {
            let (point, arm, event) = if reads.peek().is_some_and(|read|
                writes.peek().is_none_or(|write| read.1 < write.1.version))
            {
                let (index, point) = reads.next()?;
                (point, next_arm(&mut read_arms, index), Event::Read)
            } else {
                let (index, write) = writes.next()?;
                (write.version, next_arm(&mut write_arms, index), Event::Install(write.loan))
            };
            if arm != current_arm {
                if let Some(previous) = current_arm {
                    let fork = forks.get(previous.fork())?;
                    if arm.is_some_and(|new| new.fork() == previous.fork()) {
                        if previous.second() || !arm?.second() { return None; }
                        Self::split_boundary(&mut events, fork.second?)?;
                    } else {
                        Self::close_boundary(&mut events, fork, previous.second())?;
                    }
                }
                if let Some(new) = arm.filter(|new|
                    current_arm.is_none_or(|prior| prior.fork() != new.fork()))
                {
                    let fork = forks.get(new.fork())?;
                    Self::push_boundary(&mut events, fork.branch, Event::Branch)?;
                    if new.second() { Self::split_boundary(&mut events, fork.second?)?; }
                }
                current_arm = arm;
            }
            if let Some(arm) = arm {
                let fork = forks.get(arm.fork())?;
                let inside = if arm.second() {
                    fork.second? < point && point < fork.join
                } else {
                    fork.branch < point && point < fork.second.unwrap_or(fork.join)
                };
                if !inside { return None; }
            }
            Self::push_boundary(&mut events, point, event)?;
        }
        if let Some(last) = current_arm {
            Self::close_boundary(&mut events, forks.get(last.fork())?, last.second())?;
        }

        let mut reaching = Reaching::Absent;
        let mut sibling = None;
        for boundary in &mut events {
            match boundary.event {
                Event::Join => {
                    if sibling.take()? != reaching { reaching = Reaching::Diverse; }
                }
                Event::Branch => {
                    if sibling.replace(reaching).is_some() { return None; }
                }
                Event::SecondEntry => reaching = sibling.replace(reaching)?,
                Event::Read | Event::Install(_) | Event::ThenEnd => {}
            }
            boundary.reaching_before = reaching;
            if let Event::Install(loan) = boundary.event { reaching = Reaching::One(loan); }
            boundary.reaching_after = reaching;
        }
        if sibling.is_some() { return None; }

        let mut live = false;
        let mut sibling = None;
        for boundary in events.iter_mut().rev() {
            match boundary.event {
                Event::Branch => live |= sibling.take()?,
                Event::Join => {
                    if sibling.replace(live).is_some() { return None; }
                }
                Event::ThenEnd => {
                    std::mem::swap(&mut live, sibling.as_mut()?);
                }
                Event::Read | Event::Install(_) | Event::SecondEntry => {}
            }
            boundary.live_after = live;
            match boundary.event {
                Event::Read => live = true,
                Event::Install(_) => live = false,
                Event::Branch | Event::Join | Event::ThenEnd | Event::SecondEntry => {}
            }
            boundary.live_before = live;
        }
        if sibling.is_some() { return None; }
        Some(events)
    }

    fn push_boundary(events: &mut Vec<Boundary>, point: CfgPointId, event: Event) -> Option<()> {
        if events.last().is_some_and(|last| last.point >= point) { return None; }
        events.push(Boundary::new(point, event));
        Some(())
    }

    fn split_boundary(events: &mut Vec<Boundary>, second: CfgPointId) -> Option<()> {
        Self::push_boundary(events, CfgPointId(second.0.checked_sub(1)?), Event::ThenEnd)?;
        Self::push_boundary(events, second, Event::SecondEntry)
    }

    fn close_boundary(events: &mut Vec<Boundary>, fork: &Fork, on_second: bool) -> Option<()> {
        if let Some(second) = fork.second.filter(|_| !on_second) {
            Self::split_boundary(events, second)?;
        }
        Self::push_boundary(events, fork.join, Event::Join)
    }

}

fn access_facts(boundaries: &[Boundary], point: CfgPointId) -> (Reaching, bool) {
    let after = boundaries.partition_point(|boundary| boundary.point < point);
    if let Some(boundary) = boundaries.get(after).filter(|boundary| boundary.point == point) {
        return (boundary.reaching_before, boundary.live_after);
    }
    let reaching = after.checked_sub(1).and_then(|index| boundaries.get(index))
        .map_or(Reaching::Absent, |boundary| boundary.reaching_after);
    let live = boundaries.get(after).is_some_and(|boundary| boundary.live_before);
    (reaching, live)
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
            if fork.second.is_none() { *fork_index += 1; }
            matches!(&point.kind, CfgPointKind::Branch { .. })
                && point.successors.as_slice() == [next, fork.second.unwrap_or(fork.join)]
        } else if matches!(&point.kind, CfgPointKind::Branch { .. }) {
            false
        } else if let Some(fork) = fork.filter(|fork| fork.second == Some(next)) {
            *fork_index += 1;
            matches!(&point.kind, CfgPointKind::ScopeExit)
                && point.successors.as_slice() == [fork.join]
        } else if point.id == self.exit {
            fork.is_none() && point.successors.is_empty()
        } else {
            point.successors.as_slice() == [next]
        }
    }

    pub(super) fn conflicts(
        &self, owner: &ReferenceBindingId, access: CfgPointId, _incoming_count: usize,
        points: &[ReferenceFlowPoint],
        mode: AccessMode,
    ) -> Option<Vec<SourceSpan>> {
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
        let (clear_stage, refine_stage) = match &points[access.0].kind {
            CfgPointKind::Borrow { .. } => (5, 6),
            CfgPointKind::Assign { .. } => (7, 8),
            _ => (0, 1),
        };
        if !active || carriers.peek().is_none() {
            #[cfg(test)]
            tests::observe(clear_stage);
            return Some(Vec::new());
        }
        #[cfg(test)]
        tests::observe(refine_stage);
        let mut conflicts = Vec::new();
        // Carrier indices were assigned by the authored BTreeMap in binding order.
        for holder in carriers {
            let entry = &self.holders[holder];
            let Some(precise) = entry.refined.get_or_init(|| {
                #[cfg(test)]
                tests::observe(2);
                entry.holder.refine(&self.forks, self.exit)
            }) else { return None };
            let (reaching, live) = precise.facts_at(access);
            if !live { continue; }
            let issue = match reaching {
                Reaching::One(loan) if loan.owner == owner_index && mode.accepts(loan.mutable) => loan.issued,
                Reaching::Diverse => {
                    #[cfg(test)]
                    tests::observe(3);
                    return None;
                }
                Reaching::Absent | Reaching::One(_) => continue,
            };
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
        static COUNTS: std::cell::Cell<[usize; 9]> = const { std::cell::Cell::new([0; 9]) };
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
            imported_shared_readers: HashMap::new(),
            local_shared_call_families: HashMap::new(),
            exclusive_scalar_writers: HashSet::new(),
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
                specialized_policies: vec![], primary_call_points: HashMap::new(), runtime_array_read_sites: HashMap::new(), runtime_array_child_sites: Vec::new(), runtime_array_exclusive_sites: HashMap::new(), observed_runtime_allocations: None, guarded_scalar_allocation: None,
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
        let scope = builder.cursor.map_or(ReferenceScopeId(0), |id| builder.cfg.points[id.0].scope);
        let point = builder.point(CfgPointKind::Borrow { place, loan }, span, scope, origins.clone());
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
        inert_result_with_stores(prepared_existing, divergent, borrow_matrix, linear, sequential, false)
    }

    fn inert_result_with_stores(
        prepared_existing: bool, divergent: bool, borrow_matrix: bool, linear: bool, sequential: bool, stores: bool,
    ) -> FunctionReferenceResult {
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
            if sequential && divergent { roots.insert(OccurrenceOrdinal(80), NamedPlaceRootIdentity::Lexical(z.clone())); }
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
                inner: Box::new(ScalarType::I32),
                mutability: if stores && holder == &h4 {
                    ScalarReferenceMutability::Shared
                } else {
                    ScalarReferenceMutability::Mutable
                },
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
        let ordinary_store = |builder: &mut ReferenceFlowBuilder<'_>, owner: &ReferenceBindingId, start, scope| {
            let span = at(start);
            let rhs = builder.point(CfgPointKind::Evaluate(ScalarExpression::Integer {
                value: 1.into(), span,
            }), span, scope, BTreeSet::new());
            let target = builder.reference_place(
                &ScalarPlace::Name { name: "owner".into(), span }, owner.clone(), None,
            );
            let emitted = builder.emit_direct_assignment(owner.clone(), CfgPointKind::Assign {
                target, rhs_point: rhs, previous_origins: BTreeSet::new(), previous_loans: vec![],
            }, rhs, ReferenceValue::Other, span, scope).expect("typed ordinary store");
            builder.author_direct_assignment(emitted, prepared_existing);
        };
        let (unread, unread_origins) = if stores {
            loan_with_mode(&mut builder, &z, at(34), ScalarReferenceMutability::Shared)
        } else {
            loan(&mut builder, &z, at(34))
        };
        bind(&mut builder, &h4, 3, unread, unread_origins.clone());
        if stores {
            ordinary_store(&mut builder, &z, 50, root);
            builder.point(CfgPointKind::Read { binding: h4.clone() }, at(51), root, unread_origins);
            ordinary_store(&mut builder, &z, 52, root);
        }
        let divergent_origins;
        if linear {
            assert!(!divergent);
            divergent_origins = None;
            builder.point(CfgPointKind::Read { binding: x.clone() }, at(8), root, BTreeSet::new());
            builder.point(CfgPointKind::Read { binding: y.clone() }, at(9), root, BTreeSet::new());
        } else {
            let branch = builder.point(CfgPointKind::Branch { condition: ScalarExpression::Boolean { value: true, span: at(6) } }, at(6), root, BTreeSet::new());
            builder.point(CfgPointKind::ScopeEnter, at(7), arm, BTreeSet::new());
            if stores {
                ordinary_store(&mut builder, &y, 54, arm);
            }
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
        if stores {
            ordinary_store(&mut builder, &x, 55, root);
        }
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
            if divergent {
                let (rhs, origins) = loan(&mut builder, &z, at(76));
                let target = builder.reference_place(&ScalarPlace::Name { name: "h1".into(), span: at(77) }, h1.clone(), None);
                let emitted = builder.emit_direct_assignment(h1.clone(), CfgPointKind::Assign {
                    target, rhs_point: rhs, previous_origins: BTreeSet::new(), previous_loans: vec![],
                }, rhs, ReferenceValue::Checked(origins.clone()), at(77), second_arm).expect("typed second-arm assignment");
                builder.author_direct_assignment(emitted, prepared_existing);
                builder.point(CfgPointKind::Read { binding: z.clone() }, at(74), second_arm, BTreeSet::new());
                builder.point(CfgPointKind::Read { binding: h1.clone() }, at(78), second_arm, origins);
            }
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
            if divergent {
                let (rhs, origins) = loan(&mut builder, &z, at(80));
                let target = builder.reference_place(&ScalarPlace::Name { name: "h1".into(), span: at(81) }, h1.clone(), None);
                let emitted = builder.emit_direct_assignment(h1.clone(), CfgPointKind::Assign {
                    target, rhs_point: rhs, previous_origins: BTreeSet::new(), previous_loans: vec![],
                }, rhs, ReferenceValue::Checked(origins.clone()), at(81), root).expect("typed join-restoring assignment");
                builder.author_direct_assignment(emitted, prepared_existing);
                builder.point(CfgPointKind::Read { binding: z.clone() }, at(82), root, BTreeSet::new());
                builder.point(CfgPointKind::Read { binding: h1.clone() }, at(83), root, origins);
            }
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
            checked_contract_returns: vec![], whole_generic_formal: None, aggregate: aggregate::AggregateFlowResult::default(), arms: vec![],
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

    fn assert_retired_read_clears_before_refinement(result: &FunctionReferenceResult) {
        let sealed = result.proof_points.local_borrow.as_ref().unwrap();
        let points = &result.cfg.initial_points;
        let read = points.iter().find(|point| point.source_span.range == ByteSpan::new(8, 9)
            && matches!(point.kind, CfgPointKind::Read { .. })).unwrap();
        let CfgPointKind::Read { binding } = &read.kind else { unreachable!() };
        let owner = &sealed.owners[sealed.owner_index[binding]];
        assert!(owner.carriers.iter().any(|&(holder, modes)|
            modes & 2 != 0 && contains(&sealed.holders[holder].live, read.id)));
        assert!(!contains(&owner.active[1], read.id));
        let before = COUNTS.with(|counts| counts.get());
        assert_eq!(sealed.conflicts(binding, read.id, 1, points, AccessMode::Shared), Some(Vec::new()));
        let after = COUNTS.with(|counts| counts.get());
        assert_eq!(after[0], before[0] + 1);
        assert_eq!(after[1..], before[1..]);
    }

    #[test]
    fn production_builder_and_pristine_replay_replace_only_complete_local_read_verdicts() {
        let mut admitted = inert_result(true, false, false, false, false);
        assert!(admitted.proof_points.local_borrow.is_some());
        assert_retired_read_clears_before_refinement(&admitted);
        COUNTS.with(|counts| counts.set([0; 9]));
        let staged = definitive(&mut admitted);
        let counts = COUNTS.with(|counts| counts.get());
        assert_eq!(&counts[..5], &[3, 3, 2, 0, 0]);
        let again = definitive(&mut admitted);
        assert_eq!(staged, again);
        assert_eq!(COUNTS.with(|counts| counts.get()[2]), 2);
        let mut unprepared = inert_result(false, false, false, false, false);
        assert!(unprepared.proof_points.local_borrow.is_none());
        COUNTS.with(|counts| counts.set([0; 9]));
        let incumbent = definitive(&mut unprepared);
        assert_eq!(staged, incumbent);
        assert_eq!(COUNTS.with(|counts| counts.get()), [0; 9]);
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
    fn production_linear_owner_summary_stops_at_last_holder_use() {
        let mut staged = inert_result(true, false, false, true, false);
        let mut ordinary = inert_result(false, false, false, true, false);
        let sealed = staged.proof_points.local_borrow.as_ref().expect("sealed linear analysis");
        let owner = |start| sealed.owner_index.iter()
            .find(|(binding, _)| binding.declaration_span == ByteSpan::new(start, start + 1))
            .map(|(_, &index)| &sealed.owners[index]).unwrap();
        let at = |start| staged.cfg.initial_points.iter()
            .find(|point| point.source_span.range == ByteSpan::new(start, start + 1)
                && matches!(point.kind, CfgPointKind::Read { .. })).unwrap().id;
        assert!(owner(20).active[1].is_empty());
        assert!(owner(25).active[1].is_empty());
        assert!(contains(&owner(21).active[1], at(9)));
        assert!(!contains(&owner(21).active[1], at(16)));
        assert_eq!(definitive(&mut staged), definitive(&mut ordinary));
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
            if !divergent { assert_retired_read_clears_before_refinement(&admitted); }
            COUNTS.with(|counts| counts.set([0; 9]));
            let staged = definitive(&mut admitted);
            let counts = COUNTS.with(|counts| counts.get());
            let mut incumbent = inert_result(false, divergent, false, false, false);
            assert_eq!(staged, definitive(&mut incumbent));
            assert_eq!(at(&staged, 13), vec![vec![ByteSpan::new(3, 4), ByteSpan::new(15, 16)]]);
            assert_eq!(counts[4], if divergent { 2 } else { 0 });
            assert_eq!(counts[1], if divergent { 5 } else { 3 });
            assert_eq!(counts[0], if divergent { 2 } else { 3 });
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
        COUNTS.with(|counts| counts.set([0; 9]));
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
    fn production_ordinary_owner_writes_use_staged_exclusive_verdicts() {
        let at = |errors: &[ReferenceAnalysisError], start| errors.iter().filter_map(|error| match error {
            ReferenceAnalysisError::ConflictingPlace { source_span, origins }
                if source_span.range == ByteSpan::new(start, start + 1) =>
                Some(origins.iter().map(|origin| origin.range.start).collect::<Vec<_>>()),
            _ => None,
        }).collect::<Vec<_>>();
        for (linear, divergent) in [(true, false), (false, false), (false, true)] {
            let mut admitted = inert_result_with_stores(true, divergent, false, linear, false, true);
            let mut incumbent = inert_result_with_stores(false, divergent, false, linear, false, true);
            assert!(admitted.proof_points.local_borrow.is_some());
            assert!(incumbent.proof_points.local_borrow.is_none());
            COUNTS.with(|counts| counts.set([0; 9]));
            let staged = definitive(&mut admitted);
            let counts = COUNTS.with(|counts| counts.get());
            assert_eq!(staged, definitive(&mut incumbent));
            assert!(counts[7] > 0 && counts[8] > 0);
            assert_eq!(at(&staged, 50), vec![vec![34]]);
            assert!(at(&staged, 52).is_empty());
            if !linear {
                assert_eq!(at(&staged, 54), vec![vec![3, 15]]);
            }
            let mut post_join = at(&staged, 55);
            post_join.sort_unstable();
            assert_eq!(post_join, if divergent { vec![vec![5], vec![30]] } else { vec![] });
            assert_eq!(counts[3] > 0, divergent);
        }
    }

    #[test]
    fn production_straight_line_flow_seals_one_route_and_keeps_reference_diagnostics() {
        let mut admitted = inert_result(true, false, true, true, false);
        let mut incumbent = inert_result(false, false, true, true, false);
        let sealed = admitted.proof_points.local_borrow.as_ref().expect("complete linear proof");
        assert!(sealed.forks.first.is_none());
        assert!(incumbent.proof_points.local_borrow.is_none());
        assert_retired_read_clears_before_refinement(&admitted);
        COUNTS.with(|counts| counts.set([0; 9]));
        let staged = definitive(&mut admitted);
        let counts = COUNTS.with(|counts| counts.get());
        assert_eq!(staged, definitive(&mut incumbent));
        assert_eq!(&counts[..2], &[3, 2]);
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
        assert!(sealed.holders.iter().all(|holder| holder.holder.arm_runs().1.is_empty()));
        assert!(incumbent.proof_points.local_borrow.is_none());
        assert!(branch_store.proof_points.local_borrow.is_some());
        COUNTS.with(|counts| counts.set([0; 9]));
        let staged = definitive(&mut admitted);
        let counts = COUNTS.with(|counts| counts.get());
        assert_eq!(staged, definitive(&mut incumbent));
        COUNTS.with(|counts| counts.set([0; 9]));
        let overwritten = definitive(&mut branch_store);
        let overwrite_counts = COUNTS.with(|counts| counts.get());
        assert_eq!(overwritten, definitive(&mut branch_store_incumbent));
        assert!(overwrite_counts[3] > 0);
        let spans_at = |start| overwritten.iter().find_map(|error| match error {
            ReferenceAnalysisError::ConflictingPlace { source_span, origins }
                if source_span.range == ByteSpan::new(start, start + 1) =>
                Some(origins.iter().map(|span| span.range).collect::<Vec<_>>()),
            _ => None,
        });
        assert_eq!(spans_at(74), Some(vec![ByteSpan::new(76, 77)]));
        assert_eq!(spans_at(82), Some(vec![ByteSpan::new(80, 81)]));
        assert_eq!(counts[3], 0);
        assert!(counts[0] > 0 && counts[1] > 0);
        assert!(staged.iter().any(|error| matches!(error,
            ReferenceAnalysisError::ConflictingPlace { source_span, origins }
                if source_span.range == ByteSpan::new(9, 10) && origins.len() == 2)));
        assert!(!staged.iter().any(|error| matches!(error,
            ReferenceAnalysisError::ConflictingPlace { source_span, .. }
                if source_span.range == ByteSpan::new(8, 9))));
    }

    #[test]
    fn sparse_holder_boundaries_distinguish_entry_outgoing_and_bypass() {
        let at = CfgPointId;
        let old = Loan { owner: 0, issued: at(0), mutable: true };
        let taken = Loan { owner: 1, issued: at(6), mutable: false };
        let later = Loan { owner: 0, issued: at(15), mutable: true };
        let first_arm = Some(Arm::new(0, false));
        let mut holder = Holder::default();
        holder.record_install(Install { version: at(1), loan: old }, None);
        holder.record_install(Install { version: at(7), loan: taken }, first_arm);
        holder.record_install(Install { version: at(16), loan: later }, None);
        holder.record_read(at(8), first_arm);
        holder.record_read(at(9), first_arm);
        holder.record_read(at(12), None);
        holder.record_read(at(18), None);
        assert_eq!(holder.arm_runs().0.len(), 1);
        let mut forks = Forks::default();
        forks.push(Fork { branch: at(4), join: at(10), second: None });
        let facts = holder.refine(&forks, at(20)).expect("one authored fork");
        assert!(matches!(&facts, Refinement::Branching(_)));
        let cases = [
            (3, Reaching::One(old), true), // Bypass can reach the post-join read.
            (4, Reaching::One(old), true), // Branch query sees both successors.
            (5, Reaching::One(old), false), // Taken lane on its way to retirement.
            (7, Reaching::One(old), true), // Incoming version; future read starts after install.
            (8, Reaching::One(taken), true),
            (9, Reaching::One(taken), true), // Independent post-join read; not the read at 9.
            (10, Reaching::Diverse, true), // Bypass retains old issue.
            (11, Reaching::Diverse, true),
            (12, Reaching::Diverse, false), // This read does not count itself.
            (13, Reaching::Diverse, false),
            (16, Reaching::Diverse, true), // Root install still sees incoming alternatives.
            (17, Reaching::One(later), true),
            (18, Reaching::One(later), false),
            (19, Reaching::One(later), false),
        ];
        for (point, reaching, live) in cases {
            let actual = facts.facts_at(at(point));
            assert!(actual == (reaching, live), "point {point}");
        }
        assert!(!AccessMode::Shared.accepts(taken.mutable));
        assert!(AccessMode::Exclusive.accepts(taken.mutable));
        let mut linear = Holder::default();
        linear.record_install(Install { version: at(1), loan: old }, None);
        for point in 3..18 { linear.record_read(at(point), None); }
        assert!(linear.arms.is_none());
        let facts = linear.refine(&forks, at(20)).expect("linear compact lane");
        assert!(matches!(&facts, Refinement::Linear(leases) if leases.len() == 1));
        assert!(facts.facts_at(at(16)) == (Reaching::One(old), true));
        assert!(facts.facts_at(at(17)) == (Reaching::Absent, false));
    }

    #[test]
    fn sparse_explicit_arms_do_not_carry_reads_or_installs_to_siblings() {
        let at = CfgPointId;
        let first = Some(Arm::new(0, false));
        let second = Some(Arm::new(0, true));
        let incoming = Loan { owner: 0, issued: at(0), mutable: true };
        let written = Loan { owner: 1, issued: at(5), mutable: true };
        let reset = Loan { owner: 0, issued: at(20), mutable: false };
        let has = |loan| Reaching::One(loan);
        let mut forks = Forks::default();
        forks.push(Fork { branch: at(3), join: at(15), second: Some(at(10)) });
        let mut holder = Holder::default();
        holder.record_install(Install { version: at(1), loan: incoming }, None);
        holder.record_read(at(12), second);
        let facts = holder.refine(&forks, at(30)).expect("read in second arm");
        for (point, live) in [(2, true), (3, true), (5, false), (9, false), (10, true), (11, true), (12, false), (15, false)] {
            assert!(facts.facts_at(at(point)) == (has(incoming), live), "point {point}");
        }

        let mut holder = Holder::default();
        holder.record_install(Install { version: at(1), loan: incoming }, None);
        holder.record_install(Install { version: at(6), loan: written }, first);
        holder.record_read(at(8), first);
        holder.record_read(at(12), second);
        holder.record_read(at(17), None);
        holder.record_install(Install { version: at(21), loan: reset }, None);
        holder.record_read(at(24), None);
        let facts = holder.refine(&forks, at(30)).expect("both siblings and root reset");
        for (point, reaching, live) in [
            (5, has(incoming), false), (6, has(incoming), true), (7, has(written), true),
            (9, has(written), true), (10, has(incoming), true), (12, has(incoming), true),
            (14, has(incoming), true), (15, Reaching::Diverse, true),
            (17, Reaching::Diverse, false), (21, Reaching::Diverse, true),
            (22, has(reset), true), (24, has(reset), false),
        ] {
            assert!(facts.facts_at(at(point)) == (reaching, live), "point {point}");
        }
    }

    #[test]
    fn production_builder_and_pristine_scan_keep_explicit_siblings_separate() {
        let reaches = |enabled, malformed| {
            let source = SourceIdentity::new("project".into(), "package".into(), "metadata".into(), "r1".into());
            let at = |start| ByteSpan::new(start, start + 1);
            let span = ByteSpan::new(0, 100);
            let function = function(span);
            let root = ReferenceScopeId(0);
            let first = ReferenceScopeId(1);
            let second = ReferenceScopeId(2);
            let binding = |start| ReferenceBindingId {
                source: source.clone(), function_span: span, block_span: span,
                declaration_span: at(start), kind: ReferenceBindingKind::Declared,
            };
            let (owner, holder) = (binding(20), binding(21));
            let mut calls = prepared();
            let NamedPlaceRootFacts::Prepared(roots) = &mut calls.named_place_roots else { unreachable!() };
            if enabled {
                roots.insert(OccurrenceOrdinal(1), NamedPlaceRootIdentity::Lexical(owner.clone()));
            }
            let region = ConditionalRegion::Value {
                span, condition: OccurrenceOrdinal(1), then_block: BlockOrdinal(1),
                else_block: BlockOrdinal(2), resume: OccurrenceOrdinal(2),
            };
            calls.conditionals.insert(OccurrenceOrdinal(0), region.clone());
            let names = HashSet::new();
            let mut builder = ReferenceFlowBuilder::new(&source, &function, &names, &calls, &[], &[]);
            builder.binding_types.record(owner.clone(), ScalarType::I32, Some(root));
            builder.binding_types.record(holder.clone(), ScalarType::CheckedReference {
                inner: Box::new(ScalarType::I32), mutability: ScalarReferenceMutability::Mutable,
            }, Some(root));
            builder.point(CfgPointKind::Entry, span, root, BTreeSet::new());
            builder.point(CfgPointKind::ScopeEnter, span, root, BTreeSet::new());
            let (issue, origins) = loan(&mut builder, &owner, at(1));
            bind(&mut builder, &holder, 0, issue, origins.clone());
            builder.point(CfgPointKind::Read { binding: owner.clone() }, at(2), root, BTreeSet::new());
            let branch = builder.point(CfgPointKind::Branch {
                condition: ScalarExpression::Boolean { value: true, span: at(3) },
            }, at(3), root, BTreeSet::new());
            builder.point(CfgPointKind::ScopeEnter, at(4), first, BTreeSet::new());
            builder.point(CfgPointKind::Read { binding: owner.clone() }, at(5), first, BTreeSet::new());
            let first_end = builder.point(CfgPointKind::ScopeExit, at(6), first, BTreeSet::new());
            builder.author_local_scope_exit(first_end, &Default::default(), true);
            builder.cursor = Some(branch);
            builder.author_local_second_arm(CfgPointId(builder.cfg.points.len()));
            let second_entry = builder.point(CfgPointKind::ScopeEnter, at(7), second, BTreeSet::new());
            builder.point(CfgPointKind::Read { binding: owner.clone() }, at(8), second, BTreeSet::new());
            builder.point(CfgPointKind::Read { binding: holder.clone() }, at(9), second, origins);
            let second_end = builder.point(CfgPointKind::ScopeExit, at(10), second, BTreeSet::new());
            builder.author_local_scope_exit(second_end, &Default::default(), true);
            builder.cursor = None;
            let join = builder.point(CfgPointKind::Join, at(11), root, BTreeSet::new());
            builder.connect_reference_conditional(&ReferenceConditionalEdges {
                branch, terminals: [first_end, second_end], join,
            }, &region);
            builder.point(CfgPointKind::Read { binding: owner.clone() }, at(12), root, BTreeSet::new());
            let exit = builder.point(CfgPointKind::ScopeExit, span, root, BTreeSet::new());
            let mut leaving = super::super::super::control::ScopeExitBindings::default();
            leaving.insert(owner);
            leaving.insert(holder);
            builder.author_local_scope_exit(exit, &leaving, true);
            builder.proof_points.scope_exit_bindings.as_mut().unwrap().insert(exit, leaving);
            builder.seal_local_borrow();
            assert_eq!(builder.proof_points.local_borrow.is_some(), enabled);
            builder.cfg.initial_points = builder.cfg.points.clone();
            if malformed {
                builder.cfg.initial_points[first_end.0].successors = vec![second_entry];
                let sealed = builder.proof_points.local_borrow.as_ref().expect("pristine mismatch");
                assert!(!sealed.matches_edge_shape(first_end.0, &builder.cfg.initial_points[first_end.0], &mut 0));
                return None;
            }
            let mut result = FunctionReferenceResult {
                cfg: builder.cfg, proof_points: builder.proof_points,
                checked_contract_returns: vec![], whole_generic_formal: None, aggregate: aggregate::AggregateFlowResult::default(), arms: vec![],
            };
            Some(definitive(&mut result))
        };
        COUNTS.with(|counts| counts.set([0; 9]));
        let admitted = reaches(true, false).expect("exact production edges");
        assert!(COUNTS.with(|counts| counts.get()[2]) > 0);
        let incumbent = reaches(false, false).expect("incumbent verdict");
        assert_eq!(admitted, incumbent);
        let diagnostic = |start| admitted.iter().any(|error| matches!(error,
            ReferenceAnalysisError::ConflictingPlace { source_span, .. }
                if source_span.range == ByteSpan::new(start, start + 1)));
        assert!(diagnostic(2) && diagnostic(8));
        assert!(!diagnostic(5) && !diagnostic(12));
        assert!(reaches(true, true).is_none());
    }

}
