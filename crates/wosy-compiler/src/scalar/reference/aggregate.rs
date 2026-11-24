use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct AggregateObservationMode {
    literal: bool,
    legacy: bool,
}

#[derive(Default)]
pub(super) struct AggregateFlowFacts {
    pub(super) allocations: BTreeMap<ReferenceBindingId, CfgPointId>,
    pub(super) allocation_slots: BTreeMap<CfgPointId, (usize, usize)>,
    pub(super) literal_array_bind: Option<LiteralArrayBind>,
    pub(super) literal_array_write: Option<LiteralArrayWrite>,
    pub(super) literal_array_writer: Option<BlockOrdinal>,
    pub(super) literal_array_other_writer: Option<BlockOrdinal>,
    pub(super) literal_array_other_write: Option<LiteralArrayWrite>,
    pub(super) literal_array_unit: Option<LiteralArrayUnit>,
    pub(super) literal_array_value: Option<(CfgPointId, OccurrenceOrdinal)>,
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
    pub(super) secondary_owner_calls: BTreeSet<CfgPointId>,
    mixed_arm_projection: Option<MixedArmProjection>,
    pub(super) secondary_owner_outputs: BTreeMap<CfgPointId, CfgPointId>,
    pub(super) projected_reads: BTreeMap<CfgPointId, bool>,
    pub(super) scalar_receipt_reads: BTreeSet<CfgPointId>,
    pub(super) scope_exits: BTreeMap<CfgPointId, BlockOrdinal>,
    pub(super) formals: BTreeMap<usize, ReferenceBindingId>,
    pub(super) input_moves: BTreeMap<CfgPointId, (OccurrenceOrdinal, BTreeMap<usize, ArgumentProvenance>)>,
    pub(super) temporary_arguments: BTreeSet<CfgPointId>,
    pub(super) conditional_formal_moves: BTreeMap<CfgPointId, CfgPointId>,
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

#[derive(Clone, Copy)]
pub(super) struct ConditionalInputRegion {
    pub(super) expression: OccurrenceOrdinal,
    pub(super) branch: CfgPointId,
    pub(super) entries: [CfgPointId; 2],
    pub(super) terminals: [CfgPointId; 2],
    pub(super) join: CfgPointId,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ConditionalPath {
    Outside,
    Spine,
    Arm { fork: usize, side: usize },
}

// Walk each authored region once. Dense labels associate an arm point with
// its exact enclosing fork; parent links also identify the root-spine fork.
#[derive(Clone, Copy)]
struct CertifiedInputFork {
    region: ConditionalInputRegion,
    parent: Option<(usize, usize)>,
}

// Bound both recursion and ownership IR expansion. A receipt supports at
// most two consumption sites, so their ancestor paths contribute at most
// 2 * depth - 1 merges (the enclosing root fork is shared). Sequential forks
// start at depth one and do not spend one another's nesting budget.
const MAX_RECEIPT_FORK_DEPTH: usize = 32;
const MAX_MERGES_PER_RECEIPT: usize = 2 * MAX_RECEIPT_FORK_DEPTH - 1;

struct ConditionalInputCertificate {
    paths: Vec<ConditionalPath>,
    forks: Vec<CertifiedInputFork>,
    lexical_scopes: BTreeMap<usize, (CfgPointId, CfgPointId)>,
    depth_limit_reached: bool,
}

fn linear_successor(points: &[ReferenceFlowPoint], from: CfgPointId) -> Option<CfgPointId> {
    let [next] = points[from.0].successors.as_slice() else { return None };
    (*next > from).then_some(*next)
}

impl ConditionalInputCertificate {
    fn build(
        points: &[ReferenceFlowPoint], regions: &BTreeMap<CfgPointId, ConditionalInputRegion>,
        barriers: &BTreeSet<CfgPointId>,
    ) -> Self {
        let mut certificate = Self {
            paths: vec![ConditionalPath::Outside; points.len()],
            forks: Vec::new(),
            lexical_scopes: BTreeMap::new(),
            depth_limit_reached: false,
        };
        let mut entries = BTreeMap::<usize, CfgPointId>::new();
        for point in points {
            match point.kind {
                CfgPointKind::ScopeEnter => { entries.insert(point.scope.0, point.id); }
                CfgPointKind::ScopeExit => {
                    if let Some(entry) = entries.remove(&point.scope.0) {
                        certificate.lexical_scopes.insert(point.scope.0, (entry, point.id));
                    }
                }
                _ => {}
            }
        }
        let mut point = points[0].id;
        loop {
            if barriers.contains(&point) { break; }
            certificate.paths[point.0] = ConditionalPath::Spine;
            if let Some(next) = linear_successor(points, point) {
                point = next;
                continue;
            }
            let Some(region) = regions.get(&point) else { break };
            let certified = certificate.forks.len();
            if !certificate.certify_fork(points, regions, barriers, region, points[point.0].scope, None, 1) {
                // Partially walked descendants cannot authorize a transfer.
                certificate.forks.truncate(certified);
                break;
            }
            point = region.join;
        }
        certificate
    }

    fn certify_fork(
        &mut self, points: &[ReferenceFlowPoint], regions: &BTreeMap<CfgPointId, ConditionalInputRegion>,
        barriers: &BTreeSet<CfgPointId>, region: &ConditionalInputRegion,
        scope: ReferenceScopeId, parent: Option<(usize, usize)>, depth: usize,
    ) -> bool {
        if depth > MAX_RECEIPT_FORK_DEPTH {
            self.depth_limit_reached = true;
            return false;
        }
        if points[region.branch.0].scope != scope || points[region.join.0].scope != scope
            || region.branch >= region.join
            || points[region.branch.0].successors.as_slice() != region.entries
        {
            return false;
        }
        let fork = self.forks.len();
        self.forks.push(CertifiedInputFork { region: *region, parent });
        for (side, (entry, end)) in region.entries.into_iter().zip(region.terminals).enumerate() {
            if side == 1 && entry == region.join && end == region.branch {
                continue; // A Unit conditional's implicit else is the direct bypass.
            }
            let arm_scope = points[entry.0].scope;
            if entry <= region.branch || entry >= region.join || end >= region.join
                || arm_scope == scope || points[end.0].scope != arm_scope
                || points[end.0].successors.as_slice() != [region.join]
            {
                return false;
            }
            let mut point = entry;
            loop {
                if point > end || barriers.contains(&point) || self.paths[point.0] != ConditionalPath::Outside {
                    return false;
                }
                self.paths[point.0] = ConditionalPath::Arm { fork, side };
                if point == end { break; }
                if let Some(next) = linear_successor(points, point) {
                    point = next;
                } else if let Some(child) = regions.get(&point) {
                    if child.join > end || !self.certify_fork(points, regions, barriers, child, points[point.0].scope, Some((fork, side)), depth + 1) {
                        return false;
                    }
                    point = child.join;
                } else {
                    return false;
                }
            }
        }
        true
    }

    fn consuming_arm(
        &self, points: &[ReferenceFlowPoint], producer: CfgPointId, consumer: CfgPointId, exit: Option<CfgPointId>,
    ) -> Option<(usize, usize, usize)> {
        let owner_scope = points[producer.0].scope;
        let ConditionalPath::Arm { fork, side } = self.paths[consumer.0] else { return None };
        let candidate = self.forks.get(fork)?;
        if points[consumer.0].scope != points[candidate.region.entries[side].0].scope {
            return None;
        }
        let mut boundary = fork;
        while points[self.forks[boundary].region.branch.0].scope != owner_scope {
            boundary = self.forks[boundary].parent?.0;
        }
        let region = self.forks[boundary].region;
        let path = self.paths[region.branch.0];
        if producer >= region.branch || path == ConditionalPath::Outside || self.paths[producer.0] != path
            || exit.is_some_and(|exit| region.join > exit || points[exit.0].scope != owner_scope
                || self.paths[exit.0] != path)
        {
            return None;
        }
        if owner_scope != points[0].scope {
            let &(scope_entry, scope_exit) = self.lexical_scopes.get(&owner_scope.0)?;
            if scope_entry >= producer || region.join >= scope_exit
                || self.paths[scope_entry.0] != path || self.paths[scope_exit.0] != path
                || exit.is_some_and(|exit| exit != scope_exit)
            {
                return None;
            }
        }
        Some((fork, side, boundary))
    }

    fn merge_transfers(
        &self, first: (usize, usize, usize), second: Option<(usize, usize, usize)>,
    ) -> Option<Vec<(usize, [ReceiptGate; 2])>> {
        type Gates = [ReceiptGate; 2];
        // Use an explicit pending graph so sibling arms may arrive from distinct
        // child forks, but a second transfer on any single arm is rejected.
        let mut pending = BTreeMap::<usize, Gates>::new();
        let set = |(fork, side): (usize, usize), gate, pending: &mut BTreeMap<usize, Gates>| {
            let slot = &mut pending.entry(fork).or_default()[side];
            if *slot != ReceiptGate::Retained { return None; }
            *slot = gate;
            Some(())
        };
        set((first.0, first.1), ReceiptGate::Transferred, &mut pending)?;
        if let Some(second) = second {
            if first.2 != second.2 { return None; }
            set((second.0, second.1), ReceiptGate::Transferred, &mut pending)?;
        }
        let mut merges = Vec::new();
        while let Some((fork, gates)) = pending.pop_last() {
            if merges.len() == MAX_MERGES_PER_RECEIPT {
                return None;
            }
            let propagated = if gates[0] == gates[1] { gates[0] } else { ReceiptGate::Guard(fork) };
            merges.push((fork, gates));
            if fork != first.2 {
                set(self.forks[fork].parent?, propagated, &mut pending)?;
            }
        }
        Some(merges)
    }
}

#[cfg(test)]
#[test]
fn unit_input_receipt_requires_a_dominating_producer_and_an_exact_then_path() {
    let source = SourceIdentity::new("project".into(), "package".into(), "metadata".into(), "r1".into());
    let span = ByteSpan::new(0, 1);
    let point = |id, scope, successors: &[usize]| ReferenceFlowPoint {
        id: CfgPointId(id), kind: CfgPointKind::Join,
        source_span: SourceSpan::new(source.clone(), span),
        scope: ReferenceScopeId(scope), possible_origins: None,
        successors: successors.iter().copied().map(CfgPointId).collect(),
    };
    let mut points = [
        point(0, 0, &[1]), point(1, 0, &[2]), point(2, 0, &[3, 7]),
        point(3, 1, &[4]), point(4, 1, &[5]), point(5, 1, &[6]),
        point(6, 1, &[7]), point(7, 0, &[8]), point(8, 0, &[]),
    ];
    let region = ConditionalInputRegion {
        expression: OccurrenceOrdinal(1), branch: CfgPointId(2),
        entries: [CfgPointId(3), CfgPointId(7)], terminals: [CfgPointId(6), CfgPointId(2)],
        join: CfgPointId(7),
    };
    let proves = |points: &[ReferenceFlowPoint]| {
        let regions = BTreeMap::from([(region.branch, region)]);
        ConditionalInputCertificate::build(points, &regions, &BTreeSet::new())
            .consuming_arm(points, CfgPointId(1), CfgPointId(4), Some(CfgPointId(8))) == Some((0, 0, 0))
    };
    assert!(proves(&points));
    points[0].successors.push(CfgPointId(2));
    assert!(!proves(&points));
    points[0].successors.pop();
    points[3].successors.push(CfgPointId(5));
    assert!(!proves(&points));
    points[3].successors.pop();
    points[6].successors[0] = CfgPointId(8);
    assert!(!proves(&points));
}

#[cfg(test)]
#[test]
fn conditional_input_spine_distinguishes_arms_and_producer_epochs() {
    let source = SourceIdentity::new("project".into(), "package".into(), "metadata".into(), "r1".into());
    let span = ByteSpan::new(0, 1);
    let point = |id, scope, successors: &[usize]| ReferenceFlowPoint {
        id: CfgPointId(id), kind: CfgPointKind::Join,
        source_span: SourceSpan::new(source.clone(), span),
        scope: ReferenceScopeId(scope), possible_origins: None,
        successors: successors.iter().copied().map(CfgPointId).collect(),
    };
    let points = [
        point(0, 0, &[1]), point(1, 0, &[2]), point(2, 0, &[3, 4]),
        point(3, 1, &[5]), point(4, 2, &[5]), point(5, 0, &[6]),
        point(6, 0, &[7]), point(7, 0, &[8, 9]), point(8, 3, &[10]),
        point(9, 4, &[10]), point(10, 0, &[11]), point(11, 0, &[]),
    ];
    let region = |expression, branch, entries: [usize; 2], join| ConditionalInputRegion {
        expression: OccurrenceOrdinal(expression), branch: CfgPointId(branch),
        entries: entries.map(CfgPointId), terminals: entries.map(CfgPointId), join: CfgPointId(join),
    };
    let regions = BTreeMap::from([
        (CfgPointId(2), region(1, 2, [3, 4], 5)),
        (CfgPointId(7), region(2, 7, [8, 9], 10)),
    ]);
    let certificate = ConditionalInputCertificate::build(&points, &regions, &BTreeSet::new());
    let arm = |producer, consumer, exit| certificate.consuming_arm(
        &points, CfgPointId(producer), CfgPointId(consumer), Some(CfgPointId(exit)),
    );
    assert_eq!(arm(1, 3, 11), Some((0, 0, 0)));
    assert_eq!(arm(6, 8, 11), Some((1, 0, 1)));
    assert_eq!(arm(6, 9, 11), Some((1, 1, 1)));
    assert_eq!(arm(6, 3, 11), None);
    assert_eq!(arm(3, 8, 11), None);
    assert_eq!(arm(1, 8, 5), None);
}

#[cfg(test)]
#[test]
fn nested_conditional_input_coverage_rejects_overlapping_or_opaque_arms() {
    let source = SourceIdentity::new("project".into(), "package".into(), "metadata".into(), "r1".into());
    let span = ByteSpan::new(0, 1);
    let point = |id, scope, successors: &[usize]| ReferenceFlowPoint {
        id: CfgPointId(id), kind: CfgPointKind::Join,
        source_span: SourceSpan::new(source.clone(), span),
        scope: ReferenceScopeId(scope), possible_origins: None,
        successors: successors.iter().copied().map(CfgPointId).collect(),
    };
    let points = [
        point(0, 0, &[1]), point(1, 0, &[2]), point(2, 0, &[3, 8]),
        point(3, 1, &[4]), point(4, 1, &[5, 6]), point(5, 3, &[7]),
        point(6, 4, &[7]), point(7, 1, &[10]), point(8, 2, &[9]),
        point(9, 2, &[10]), point(10, 0, &[11]), point(11, 0, &[]),
    ];
    let region = |expression, branch, entries: [usize; 2], terminals: [usize; 2], join| ConditionalInputRegion {
        expression: OccurrenceOrdinal(expression), branch: CfgPointId(branch),
        entries: entries.map(CfgPointId), terminals: terminals.map(CfgPointId), join: CfgPointId(join),
    };
    let regions = BTreeMap::from([
        (CfgPointId(2), region(1, 2, [3, 8], [7, 9], 10)),
        (CfgPointId(4), region(2, 4, [5, 6], [5, 6], 7)),
    ]);
    let arm = |certificate: &ConditionalInputCertificate, consumer| certificate.consuming_arm(
        &points, CfgPointId(1), CfgPointId(consumer), Some(CfgPointId(11)),
    );
    let certificate = ConditionalInputCertificate::build(&points, &regions, &BTreeSet::new());
    assert_eq!(arm(&certificate, 5), Some((1, 0, 0)));
    assert_eq!(arm(&certificate, 6), Some((1, 1, 0)));
    assert_eq!(certificate.merge_transfers((1, 0, 0), Some((1, 1, 0))), Some(vec![
        (1, [ReceiptGate::Transferred, ReceiptGate::Transferred]),
        (0, [ReceiptGate::Transferred, ReceiptGate::Retained]),
    ]));
    assert!(certificate.merge_transfers((0, 0, 0), Some((1, 0, 0))).is_none());
    let opaque = ConditionalInputCertificate::build(&points, &regions, &BTreeSet::from([CfgPointId(4)]));
    assert_eq!(arm(&opaque, 5), None);
    assert_eq!(arm(&opaque, 9), None);
}

#[cfg(test)]
#[test]
fn conditional_input_nesting_budget_bounds_merge_work_without_limiting_serial_forks() {
    let source = SourceIdentity::new("project".into(), "package".into(), "metadata".into(), "r1".into());
    let span = ByteSpan::new(0, 1);
    let point = |id, scope| ReferenceFlowPoint {
        id: CfgPointId(id), kind: CfgPointKind::Join,
        source_span: SourceSpan::new(source.clone(), span),
        scope: ReferenceScopeId(scope), possible_origins: None, successors: Vec::new(),
    };
    let chain = |depth, serial| {
        let mut points = vec![point(0, 0), point(1, 0)];
        points[0].successors.push(CfgPointId(1));
        let mut regions = BTreeMap::new();
        let mut tail = CfgPointId(1);
        let mut last_consumer = tail;
        for _ in 0..serial {
            let first = points.len();
            for scope in 0..=depth {
                let id = points.len();
                points.push(point(id, scope));
            }
            let consumer = points.len() - 1;
            last_consumer = CfgPointId(consumer);
            let mut then_end = last_consumer;
            for scope in (0..depth).rev() {
                let branch = CfgPointId(first + scope);
                let then_entry = CfgPointId(branch.0 + 1);
                let join = CfgPointId(points.len());
                points.push(point(join.0, scope));
                points[branch.0].successors.extend([then_entry, join]);
                points[then_end.0].successors.push(join);
                regions.insert(branch, ConditionalInputRegion {
                    expression: OccurrenceOrdinal(branch.0), branch,
                    entries: [then_entry, join], terminals: [then_end, branch], join,
                });
                then_end = join;
            }
            points[tail.0].successors.push(CfgPointId(first));
            tail = then_end;
        }
        (points, regions, last_consumer, tail)
    };
    let classify = |depth, serial| {
        let (points, regions, consumer, exit) = chain(depth, serial);
        let certificate = ConditionalInputCertificate::build(&points, &regions, &BTreeSet::new());
        let work = certificate.consuming_arm(&points, CfgPointId(1), consumer, Some(exit))
            .and_then(|site| certificate.merge_transfers(site, None))
            .map(|merges| merges.len());
        (certificate.depth_limit_reached, work)
    };
    assert_eq!(classify(MAX_RECEIPT_FORK_DEPTH, 1), (false, Some(MAX_RECEIPT_FORK_DEPTH)));
    assert_eq!(classify(MAX_RECEIPT_FORK_DEPTH + 1, 1), (true, None));
    assert_eq!(classify(1, MAX_MERGES_PER_RECEIPT + 1), (false, Some(1)));
}

#[cfg(test)]
#[test]
fn lexical_receipt_proof_stops_at_its_birth_scope_and_rejects_sibling_routes() {
    let source = SourceIdentity::new("project".into(), "package".into(), "metadata".into(), "r1".into());
    let span = ByteSpan::new(0, 1);
    let point = |id, scope, kind, successors: &[usize]| ReferenceFlowPoint {
        id: CfgPointId(id), kind, source_span: SourceSpan::new(source.clone(), span),
        scope: ReferenceScopeId(scope), possible_origins: None,
        successors: successors.iter().copied().map(CfgPointId).collect(),
    };
    use CfgPointKind::{Join, ScopeEnter, ScopeExit};
    let points = [
        point(0, 0, ScopeEnter, &[1]), point(1, 0, Join, &[2]),
        point(2, 0, Join, &[3, 10]), point(3, 1, ScopeEnter, &[4]),
        point(4, 1, Join, &[5]), point(5, 1, Join, &[6, 7]),
        point(6, 2, ScopeExit, &[8]), point(7, 3, ScopeExit, &[8]),
        point(8, 1, Join, &[9]), point(9, 1, ScopeExit, &[13]),
        point(10, 4, ScopeEnter, &[11]), point(11, 4, Join, &[12]),
        point(12, 4, ScopeExit, &[13]), point(13, 0, Join, &[14]), point(14, 0, ScopeExit, &[]),
    ];
    let region = |branch, entries: [usize; 2], terminals: [usize; 2], join| ConditionalInputRegion {
        expression: OccurrenceOrdinal(branch), branch: CfgPointId(branch),
        entries: entries.map(CfgPointId), terminals: terminals.map(CfgPointId), join: CfgPointId(join),
    };
    let regions = BTreeMap::from([
        (CfgPointId(2), region(2, [3, 10], [9, 12], 13)),
        (CfgPointId(5), region(5, [6, 7], [6, 7], 8)),
    ]);
    let certificate = ConditionalInputCertificate::build(&points, &regions, &BTreeSet::new());
    let classify = |producer, consumer, exit: Option<usize>| {
        certificate.consuming_arm(&points, CfgPointId(producer), CfgPointId(consumer), exit.map(CfgPointId))
    };
    assert_eq!(classify(1, 6, Some(14)), Some((1, 0, 0)));
    assert_eq!(classify(4, 6, Some(9)), Some((1, 0, 1)));
    assert_eq!(classify(4, 7, None), Some((1, 1, 1)));
    assert_eq!(classify(4, 6, Some(8)), None);
    assert_eq!(classify(4, 6, Some(14)), None);
    assert_eq!(classify(4, 11, None), None);
    assert_eq!(certificate.merge_transfers((1, 0, 1), Some((1, 1, 1))), Some(vec![
        (1, [ReceiptGate::Transferred, ReceiptGate::Transferred]),
    ]));
}

impl AggregateFlowFacts {
    pub(super) fn record_mixed_arm_projection(
        &mut self, conditional: OccurrenceOrdinal, owner_projection: CfgPointId, owner_lane: usize,
        blocks: [BlockOrdinal; 2], outputs: [CfgPointId; 2],
    ) {
        let slots = &self.allocation_slots;
        if slots.len() == 2 && blocks[0] != blocks[1]
            && slots.values().any(|(block, _)| *block == blocks[0].0)
            && slots.values().any(|(block, _)| *block == blocks[1].0)
        {
            self.mixed_arm_projection = Some(MixedArmProjection {
                conditional, owner_projection, owner_lane, blocks, outputs,
            });
        }
    }

    pub(super) fn seal_mixed_branch_return(
        &self, cfg: &ScalarReferenceCfg, root_returns: &[ReferenceFlowPoint], result: &mut AggregateFlowResult,
    ) {
        let Some(shape) = self.mixed_arm_projection else { return };
        let [MixedArmBacking::Exact(left), MixedArmBacking::Exact(right)] = result.mixed_arm_backings else { return };
        if left == right || !result.unsupported.is_empty() || !result.unproved_receipt_loans.is_empty() {
            return;
        }
        let Some(owner_return) = root_returns.get(shape.owner_lane).filter(|point| matches!(point.kind,
            CfgPointKind::ReturnOutput { output, value_point, checked: false }
                if output == shape.owner_lane && value_point == shape.owner_projection))
        else { return };
        let [then_point, else_point] = shape.outputs;
        if !matches!(cfg.points[shape.owner_projection.0].kind,
            CfgPointKind::ConditionalOutput { then_point: first, else_point: second }
                if first == then_point && second == else_point)
        {
            return;
        }
        let Some(owners) = result.returns.get(&owner_return.id) else { return };
        if owners.len() != 2 || !owners.contains(&Some(OwnerSource::Allocation(left)))
            || !owners.contains(&Some(OwnerSource::Allocation(right)))
        {
            return;
        }
        let [Some(&(left_block, left_item)), Some(&(right_block, right_item))] =
            [self.allocation_slots.get(&left), self.allocation_slots.get(&right)] else { return };
        if [left_block, right_block] != shape.blocks.map(|block| block.0) {
            return;
        }
        result.mixed_branch_return = Some(Box::new(MixedBranchReturn {
            shape,
            allocations: [
                BlockPosition { block: shape.blocks[0], item: left_item },
                BlockPosition { block: shape.blocks[1], item: right_item },
            ],
        }));
    }

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FreshArrayExtent {
    Literal(u64),
    Formal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReplacementArrayExtent {
    Literal(u64),
    Argument { occurrence: usize },
}

/// Exact validated extent at an ordinary root allocation, not a type extent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct LiteralArrayBind {
    pub(super) bind: CfgPointId,
    pub(super) placeholder: CfgPointId,
    pub(super) position: BlockPosition,
    pub(super) length_occurrence: OccurrenceOrdinal,
    pub(super) extent: FreshArrayExtent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct LiteralArrayArgument {
    pub(super) read: CfgPointId,
    pub(super) occurrence: OccurrenceOrdinal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct LiteralArrayWrite {
    pub(super) rhs: CfgPointId,
    pub(super) writer: CfgPointId,
    pub(super) producer: OccurrenceOrdinal,
    pub(super) destination: OccurrenceOrdinal,
    pub(super) argument: Option<LiteralArrayArgument>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct LiteralArrayRetained {
    pub(super) entry: CfgPointId,
    pub(super) exit: CfgPointId,
    pub(super) block: BlockOrdinal,
    pub(super) scope: ReferenceScopeId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ArrayWriterSide {
    Then,
    Else,
}

impl ArrayWriterSide {
    pub(crate) fn split<T>(self, then: T, otherwise: T) -> (T, T) {
        match self {
            Self::Then => (then, otherwise),
            Self::Else => (otherwise, then),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct LiteralArrayUnit {
    pub(super) expression: OccurrenceOrdinal,
    pub(super) condition: CfgPointId,
    pub(super) operands: Option<[CfgPointId; 2]>,
    pub(super) branch: CfgPointId,
    pub(super) writer_side: ArrayWriterSide,
    pub(super) writer_entry: CfgPointId,
    pub(super) writer_exit: CfgPointId,
    pub(super) writer_block: BlockOrdinal,
    pub(super) writer_scope: ReferenceScopeId,
    // The other arm is empty unless its exact writer is separately proved.
    pub(super) retained: Option<LiteralArrayRetained>,
    pub(super) other_write: Option<LiteralArrayWrite>,
    pub(super) join: CfgPointId,
}

#[derive(Clone, Copy, Default, PartialEq)]
enum LiteralArrayPhase {
    #[default]
    BeforeBind,
    Original,
    Read(CfgPointId),
    Moved(CfgPointId),
    Output(OccurrenceOrdinal),
    ProducerExit(OccurrenceOrdinal),
    ConditionRead,
    ConditionLiteral,
    Condition,
    Forked(CfgPointId),
    WriterOriginal,
    WriterExit(CfgPointId),
    RetainedOriginal,
    RetainedExit(CfgPointId),
    JoinedOriginal,
    JoinedFresh,
    JoinedOtherFresh,
    ArgumentRead(CfgPointId),
    Call(CfgPointId),
    Replaced,
    Finished,
    Unknown,
}

impl LiteralArrayUnit {
    fn branch_successors(&self) -> [CfgPointId; 2] {
        let retained = self.retained.map_or(self.join, |edge| edge.entry);
        let (then, otherwise) = self.writer_side.split(self.writer_entry, retained);
        [then, otherwise]
    }

    fn arrive_at_join(&self, point: CfgPointId, source: LiteralArrayPhase) -> LiteralArrayPhase {
        if point != self.join {
            return LiteralArrayPhase::Unknown;
        }
        match source {
            LiteralArrayPhase::Forked(branch) if branch == self.branch
                && self.writer_side == ArrayWriterSide::Then && self.retained.is_none() =>
                LiteralArrayPhase::JoinedOriginal,
            LiteralArrayPhase::RetainedExit(exit) if self.other_write.is_none()
                && self.retained.is_some_and(|retained| retained.exit == exit) => LiteralArrayPhase::JoinedOriginal,
            LiteralArrayPhase::WriterExit(exit) if exit == self.writer_exit => LiteralArrayPhase::JoinedFresh,
            LiteralArrayPhase::WriterExit(exit) if self.other_write.is_some()
                && self.retained.is_some_and(|other| other.exit == exit) => LiteralArrayPhase::JoinedOtherFresh,
            _ => LiteralArrayPhase::Unknown,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum LiteralArrayRoute {
    FreshLeaf {
        allocation: LiteralArrayBind,
        output: OccurrenceOrdinal,
    },
    Replacement {
        original: LiteralArrayBind,
        write: LiteralArrayWrite,
        exit: CfgPointId,
        block: BlockOrdinal,
        unit: Option<LiteralArrayUnit>,
    },
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
    ConditionalFormal(usize),
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MixedOwnerResultLanes {
    pub(crate) owner: usize,
    pub(crate) scalar: usize,
}

pub(crate) fn mixed_owner_result_lanes(first: &ScalarType, second: &ScalarType) -> Option<MixedOwnerResultLanes> {
    match (first, second) {
        (ScalarType::Struct(_), scalar) if ordinary_value(scalar) => Some(MixedOwnerResultLanes { owner: 0, scalar: 1 }),
        (scalar, ScalarType::Struct(_)) if ordinary_value(scalar) => Some(MixedOwnerResultLanes { owner: 1, scalar: 0 }),
        _ => None,
    }
}

pub(crate) fn mixed_owned_struct_result(outputs: &ScalarOutputSequence) -> Option<MixedOwnerResultLanes> {
    let [first, second] = outputs.outputs.as_slice() else { return None };
    mixed_owner_result_lanes(&first.ty, &second.ty)
}

#[cfg(test)]
#[test]
fn mixed_owner_lanes_do_not_admit_other_result_shapes() {
    let owner = ScalarType::Struct(ScalarStructId {
        source: SourceIdentity::new("project".into(), "package".into(), "metadata".into(), "r1".into()),
        index: 0,
    });
    let scalar = ScalarType::U64;
    assert_eq!(
        mixed_owner_result_lanes(&owner, &scalar),
        Some(MixedOwnerResultLanes { owner: 0, scalar: 1 }),
    );
    assert_eq!(mixed_owner_result_lanes(&scalar, &owner), Some(MixedOwnerResultLanes { owner: 1, scalar: 0 }));
    assert!(mixed_owner_result_lanes(&owner, &owner).is_none());
    assert!(mixed_owner_result_lanes(&scalar, &scalar).is_none());
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
    array_last_recommit: Option<CfgPointId>,
    array_dependencies: BTreeSet<CfgPointId>,
    local_array: Option<LocalArrayLife>,
    literal_array_phase: LiteralArrayPhase,
    literal_array_writer_arm: Option<ArrayWriterSide>,
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
    possible: BTreeMap<OwnerSource, Option<ReferenceBindingId>>,
    // None means no path has been observed, not a path with no live receipts.
    always: Option<BTreeSet<LiveReceipt>>,
}

impl ScopeEndpoint {
    fn observe(&mut self, live: BTreeSet<LiveReceipt>) {
        for (owner, binding) in &live {
            let witness = self.possible.entry(*owner).or_insert_with(|| Some(binding.clone()));
            if witness.as_ref().is_some_and(|previous| previous != binding) {
                *witness = None; // More than one binding has carried this owner at the endpoint.
            }
        }
        if let Some(always) = &mut self.always {
            always.retain(|receipt| live.contains(receipt));
        } else {
            self.always = Some(live);
        }
    }

    fn receipt(&self, owner: OwnerSource) -> Option<bool> {
        self.possible.get(&owner).map(|witness| self.always_retains(owner, witness.as_ref()))
    }

    fn always_retains(&self, owner: OwnerSource, witness: Option<&ReferenceBindingId>) -> bool {
        witness.is_some_and(|binding| self.always.as_ref()
            .is_some_and(|always| always.contains(&(owner, binding.clone()))))
    }

    fn has_conditional_unique_receipt(&self, owner: OwnerSource) -> bool {
        self.possible.get(&owner).and_then(Option::as_ref)
            .is_some_and(|binding| self.always.is_some() && !self.always_retains(owner, Some(binding)))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RecommittedArrayExtent {
    Literal(u64),
    Formal(usize),
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ArrayReturnPath {
    dependencies: BTreeSet<CfgPointId>,
    tail_call: Option<CfgPointId>,
    last_producer: Option<CfgPointId>,
}

fn grounded_array_return_dependencies(
    body: BodyKey,
    paths: &BTreeSet<Option<ArrayReturnPath>>,
    mut resolve: impl FnMut(CfgPointId) -> Option<BodyKey>,
) -> Option<BTreeSet<BodyKey>> {
    let mut grounded = false;
    let mut dependencies = BTreeSet::new();
    for path in paths {
        let path = path.as_ref()?;
        if path.tail_call.is_some_and(|call| !path.dependencies.contains(&call))
            || path.last_producer.is_some_and(|call| !path.dependencies.contains(&call))
        {
            return None;
        }
        let mut self_call = false;
        for &point in &path.dependencies {
            let target = resolve(point)?;
            if path.tail_call == Some(point) && target != body {
                return None;
            }
            self_call |= target == body;
            dependencies.insert(target);
        }
        grounded |= path.tail_call.is_none() && !self_call;
    }
    grounded.then_some(dependencies)
}

#[derive(Default)]
struct LiteralArrayJoin {
    retained: bool,
    replaced: bool,
    other_replaced: bool,
}

impl LiteralArrayJoin {
    fn observe_exit(&mut self, phase: LiteralArrayPhase) {
        match phase {
            LiteralArrayPhase::JoinedOriginal => self.retained = true,
            LiteralArrayPhase::JoinedFresh => self.replaced = true,
            LiteralArrayPhase::JoinedOtherFresh => self.other_replaced = true,
            _ => {}
        }
    }

    fn complete(&self, dual: bool) -> bool {
        self.replaced && if dual {
            self.other_replaced && !self.retained
        } else {
            self.retained && !self.other_replaced
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum MixedArmBacking {
    #[default]
    Unseen,
    Exact(CfgPointId),
    Unproved,
}

impl MixedArmBacking {
    fn observe(&mut self, backing: Option<CfgPointId>) {
        *self = match (*self, backing) {
            (Self::Unseen, Some(point)) => Self::Exact(point),
            (Self::Exact(known), Some(point)) if known == point => Self::Exact(point),
            _ => Self::Unproved,
        };
    }
}

#[cfg(test)]
#[test]
fn mixed_arm_backing_never_recovers_from_conflicting_evidence() {
    let mut arm = MixedArmBacking::Unseen;
    arm.observe(Some(CfgPointId(1)));
    arm.observe(Some(CfgPointId(1)));
    assert_eq!(arm, MixedArmBacking::Exact(CfgPointId(1)));
    arm.observe(Some(CfgPointId(2)));
    arm.observe(Some(CfgPointId(1)));
    assert_eq!(arm, MixedArmBacking::Unproved);

    let mut arm = MixedArmBacking::Unseen;
    arm.observe(None);
    arm.observe(Some(CfgPointId(1)));
    assert_eq!(arm, MixedArmBacking::Unproved);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MixedArmProjection {
    conditional: OccurrenceOrdinal,
    owner_projection: CfgPointId,
    owner_lane: usize,
    blocks: [BlockOrdinal; 2],
    outputs: [CfgPointId; 2],
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MixedBranchReturn {
    shape: MixedArmProjection,
    allocations: [BlockPosition; 2],
}

#[derive(Default)]
pub(super) struct AggregateFlowResult {
    pub(super) conditional_input_regions: BTreeMap<CfgPointId, ConditionalInputRegion>,
    pub(super) conditional_input_barriers: BTreeSet<CfgPointId>,
    direct_receipt_bindings: BTreeMap<CfgPointId, (CfgPointId, ReferenceBindingId)>,
    literal_array_route: Option<LiteralArrayRoute>,
    literal_array_unknown: bool,
    literal_array_join: LiteralArrayJoin,
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
    pub(super) fresh_recommit_leaf: Option<RecommittedArrayExtent>,
    pub(super) array_rebinds: BTreeMap<CfgPointId, (CfgPointId, ReferenceBindingId, OccurrenceOrdinal)>,
    pub(super) local_array_failure_arms: BTreeMap<usize, Option<CfgPointId>>,
    pub(super) allocation_slots: BTreeMap<CfgPointId, (usize, usize)>,
    pub(super) calls: BTreeMap<CfgPointId, OccurrenceOrdinal>,
    pub(super) mixed_result_calls: BTreeSet<CfgPointId>,
    pub(super) mixed_result_bindings: BTreeSet<CfgPointId>,
    pub(super) mixed_forward: Option<CfgPointId>,
    pub(super) mixed_arm_backings: [MixedArmBacking; 2],
    mixed_branch_return: Option<Box<MixedBranchReturn>>,
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
    conditional_formal_forward: Option<(usize, CfgPointId, usize, usize, CfgPointId)>,
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
    #[cfg(test)]
    pub(super) fn mixed_branch_return_locations(
        &self,
    ) -> Option<(OccurrenceOrdinal, [CfgPointId; 2], [BlockPosition; 2])> {
        let plan = self.mixed_branch_return.as_ref()?;
        Some((plan.shape.conditional, plan.shape.outputs, plan.allocations))
    }

    pub(super) fn admits_mixed_branch_return(&self) -> bool {
        self.mixed_branch_return.is_some()
    }

    pub(super) fn proves_mixed_local_owner(&self, cfg: &ScalarReferenceCfg, lanes: MixedOwnerResultLanes) -> bool {
        let mut owners = self.return_owners(cfg, Some(lanes.owner));
        let Some(Some(owner @ OwnerSource::Allocation(_))) = owners.next() else { return false };
        owners.all(|current| current == Some(owner))
    }

    fn return_owners<'a>(&'a self, cfg: &'a ScalarReferenceCfg, owner_lane: Option<usize>)
        -> impl Iterator<Item = Option<OwnerSource>> + 'a
    {
        self.returns.iter().filter(move |(point, _)| owner_lane.is_none_or(|owner| matches!(cfg.points[point.0].kind,
            CfgPointKind::ReturnOutput { output, .. } if output == owner)))
            .flat_map(|(_, alternatives)| alternatives.iter().copied())
    }

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

    fn proved_literal_array_route_ref(&self) -> Option<&LiteralArrayRoute> {
        let route = self.literal_array_route.as_ref()?;
        let complete = match route {
            LiteralArrayRoute::Replacement { unit: Some(unit), .. } =>
                self.literal_array_join.complete(unit.other_write.is_some()),
            _ => true,
        };
        (!self.literal_array_unknown && complete).then_some(route)
    }

    fn proved_literal_array_route(&self) -> Option<LiteralArrayRoute> {
        self.proved_literal_array_route_ref().copied()
    }

    fn proved_literal_array_caller(&self, key: BodyKey, is_monomorphic: bool) -> Option<&LiteralArrayRoute> {
        if !is_monomorphic || key.2.is_some() {
            return None;
        }
        self.proved_literal_array_route_ref().filter(|route| matches!(route, LiteralArrayRoute::Replacement { .. }))
    }

    pub(super) fn has_observations(&self) -> bool {
        self.proved_literal_array_route_ref().is_some()
            || (self.checked_producer_authoritative && !self.checked_producer_backing.is_empty())
            || self.checked_backing_assignment.is_some() || self.checked_backing_forward.is_some()
            || self.scalar_descriptor_assignment.is_some() || self.fresh_recommit_leaf.is_some() || !self.array_returns.is_empty() || !self.array_rebinds.is_empty()
            || self.returning_leaf || !self.calls.is_empty() || !self.mixed_result_calls.is_empty() || !self.returns.is_empty() || !self.formals.is_empty() || !self.inputs.is_empty() || !self.raw_terminals.is_empty() || !self.read_formals.is_empty()
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

    // During CFG construction the final route is not yet known. Preserve
    // legacy scope metadata even when the private observer never enrolls.
    pub(super) fn needs_scope_exit_metadata(&self) -> bool {
        self.literal_array_bind.is_some() || self.observes_legacy_values()
    }

    fn observes_legacy_values(&self) -> bool {
        self.array_formal.is_some() || !self.array_calls.is_empty()
            || self.mixed_arm_projection.is_some()
            || !self.checked_producer_arms.is_empty() || !self.views.is_empty() || !self.calls.is_empty() || !self.formals.is_empty() || !self.input_moves.is_empty() || !self.raw_formals.is_empty()
    }

    // Select observers after CFG construction; this is only a demand gate,
    // not authority to transfer or release an allocation.
    pub(super) fn observation_mode(&self) -> Option<AggregateObservationMode> {
        let legacy = self.observes_legacy_values();
        let literal = self.literal_array_bind.is_some_and(|allocation|
            self.allocations.len() == 1 && self.allocations.values().next() == Some(&allocation.bind))
            && match (self.literal_array_value, self.literal_array_write, self.literal_array_unit) {
                (Some(_), None, None) => self.literal_array_writer.is_none(),
                (None, Some(_), None) => self.literal_array_writer.is_none(),
                (None, Some(_), Some(_)) => true,
                _ => false,
            };
        (literal || legacy).then_some(AggregateObservationMode { literal, legacy })
    }

    fn visit_literal_array(
        &self,
        point: &ReferenceFlowPoint,
        state: &mut AggregateEdgeState,
        result: &mut AggregateFlowResult,
    ) {
        let Some(allocation) = self.literal_array_bind else { return };
        let Some((binding, &bind)) = self.allocations.first_key_value() else { return };
        if self.allocations.len() != 1 || bind != allocation.bind {
            result.literal_array_unknown = true;
            return;
        }
        let mut completed = None;
        let unit = self.literal_array_unit;
        let root = point.scope == ReferenceScopeId(0);
        let writer_scope = unit.map_or(root, |unit| point.scope == unit.writer_scope);
        use LiteralArrayPhase::*;
        let retained = unit.and_then(|unit| unit.retained);
        let retained_scope = retained.is_some_and(|retained| point.scope == retained.scope);
        let other_write = unit.and_then(|unit| unit.other_write);
        let active_write = match (unit, state.literal_array_writer_arm) {
            (None, None) if root => self.literal_array_write,
            (Some(unit), Some(side)) if side == unit.writer_side && writer_scope =>
                self.literal_array_write,
            (Some(unit), Some(ArrayWriterSide::Else))
                if unit.writer_side == ArrayWriterSide::Then && retained_scope => other_write,
            _ => None,
        };
        let next = if (!root && !writer_scope && !retained_scope)
            || (point.successors.len() > 1 && !unit.is_some_and(|unit| unit.branch == point.id))
        {
            Unknown
        } else {
            match (state.literal_array_phase, &point.kind) {
                (BeforeBind, CfgPointKind::Entry | CfgPointKind::ScopeEnter) => BeforeBind,
                (BeforeBind, CfgPointKind::Evaluate(_)) if point.id == allocation.placeholder => BeforeBind,
                (BeforeBind, CfgPointKind::Bind { binding: actual })
                    if point.id == allocation.bind && actual == binding => Original,
                (Original, CfgPointKind::Read { .. }) if root && unit.is_some_and(|unit|
                    unit.operands.is_some_and(|[read, literal]|
                        read == point.id && point.successors.as_slice() == [literal])) => ConditionRead,
                (ConditionRead, CfgPointKind::Evaluate(ScalarExpression::Integer { .. }))
                    if root && unit.is_some_and(|unit| unit.operands.is_some_and(|[_, literal]|
                        literal == point.id && point.successors.as_slice() == [unit.condition])) =>
                    ConditionLiteral,
                (ConditionLiteral, CfgPointKind::Evaluate(ScalarExpression::Binary {
                    operator: BinaryOperator::Equal, .. }))
                    if root && unit.is_some_and(|unit| unit.condition == point.id
                        && point.successors.as_slice() == [unit.branch]) => Condition,
                (Original, CfgPointKind::Evaluate(ScalarExpression::Boolean { .. }) | CfgPointKind::Read { .. })
                    if root && unit.is_some_and(|unit| unit.operands.is_none()
                        && unit.condition == point.id
                        && point.successors.as_slice() == [unit.branch]) => Condition,
                (Condition, CfgPointKind::Branch { .. })
                    if root && unit.is_some_and(|unit| unit.branch == point.id
                        && point.successors.as_slice() == unit.branch_successors()) =>
                    Forked(point.id),
                (Forked(branch), CfgPointKind::ScopeEnter)
                    if unit.is_some_and(|unit| unit.branch == branch && unit.writer_entry == point.id
                        && point.scope == unit.writer_scope) =>
                {
                    state.literal_array_writer_arm = unit.map(|unit| unit.writer_side);
                    WriterOriginal
                }
                (Forked(branch), CfgPointKind::ScopeEnter)
                    if unit.is_some_and(|unit| unit.branch == branch)
                        && other_write.is_some()
                        && retained.is_some_and(|other| other.entry == point.id && retained_scope) =>
                {
                    state.literal_array_writer_arm = Some(ArrayWriterSide::Else);
                    WriterOriginal
                }
                (Forked(branch), CfgPointKind::ScopeEnter)
                    if unit.is_some_and(|unit| unit.branch == branch)
                        && other_write.is_none()
                        && retained.is_some_and(|retained| retained.entry == point.id
                            && retained_scope && point.successors.as_slice() == [retained.exit]) =>
                    RetainedOriginal,
                (RetainedOriginal, CfgPointKind::ScopeExit)
                    if retained.is_some_and(|retained| retained.exit == point.id && retained_scope)
                        && unit.is_some_and(|unit| point.successors.as_slice() == [unit.join]) =>
                    RetainedExit(point.id),
                (Replaced, CfgPointKind::ScopeExit)
                    if writer_scope && unit.is_some_and(|unit| unit.writer_exit == point.id
                        && state.literal_array_writer_arm == Some(unit.writer_side)
                        && point.successors.as_slice() == [unit.join]) => WriterExit(point.id),
                (Replaced, CfgPointKind::ScopeExit)
                    if other_write.is_some() && retained_scope
                        && state.literal_array_writer_arm == Some(ArrayWriterSide::Else)
                        && retained.is_some_and(|other| other.exit == point.id)
                        && unit.is_some_and(|unit| point.successors.as_slice() == [unit.join]) =>
                    WriterExit(point.id),
                (source @ (Forked(_) | WriterExit(_) | RetainedExit(_)), CfgPointKind::Join) if root =>
                    unit.map_or(Unknown, |unit| unit.arrive_at_join(point.id, source)),
                (Original, CfgPointKind::Read { binding: actual }) if actual == binding
                    && unit.is_none() && self.literal_array_write.is_none() => Read(point.id),
                (Read(value), CfgPointKind::Move { place, owner, ty: ScalarType::RuntimeArray { .. } })
                    if owner == binding && &place.binding == binding && place.projections.is_empty() => Moved(value),
                (Moved(value), CfgPointKind::ReturnOutput { output: 0, value_point, checked: false })
                    if value == *value_point =>
                {
                    match self.literal_array_value {
                        Some((recorded, output)) if recorded == value => Output(output),
                        _ => Unknown,
                    }
                }
                (Output(output), CfgPointKind::ScopeExit) => ProducerExit(output),
                (ProducerExit(output), CfgPointKind::Return) if point.successors.is_empty() => {
                    completed = Some(LiteralArrayRoute::FreshLeaf { allocation, output });
                    Finished
                }
                (Original | WriterOriginal, CfgPointKind::Read { .. })
                    if active_write.is_some_and(|write| write.argument.is_some_and(|argument| argument.read == point.id)) =>
                    ArgumentRead(point.id),
                (phase @ (Original | WriterOriginal | ArgumentRead(_)), CfgPointKind::Call { output: 0,
                    target: ResolvedCallTarget::ModuleCallable {
                        concrete: ConcreteCallSelection::Declaration, ..
                    }, argument_points, .. })
                    if active_write.is_some_and(|write| write.rhs == point.id
                        && match (phase, write.argument.map(|argument| argument.read), argument_points.as_slice()) {
                            (Original | WriterOriginal, None, []) => true,
                            (ArgumentRead(seen), Some(expected), [actual]) => seen == expected && seen == *actual,
                            _ => false,
                        }) => Call(point.id),
                (Call(call), CfgPointKind::Assign { target, rhs_point, .. })
                    if call == *rhs_point && &target.binding == binding && target.projections.is_empty()
                        && active_write.is_some_and(|write|
                            write.rhs == *rhs_point && write.writer == point.id) => Replaced,
                (phase @ (Replaced | JoinedOriginal | JoinedFresh | JoinedOtherFresh), CfgPointKind::ScopeExit)
                    if root && point.successors.is_empty()
                        && (unit.is_none() == (phase == Replaced)) =>
                {
                    if let (Some(write), Some(&block)) =
                        (self.literal_array_write, self.scope_exits.get(&point.id))
                    {
                        completed = Some(LiteralArrayRoute::Replacement { original: allocation,
                            write, exit: point.id, block, unit });
                        result.literal_array_join.observe_exit(phase);
                        Finished
                    } else {
                        Unknown
                    }
                }
                _ => Unknown,
            }
        };
        if next == Unknown || point.successors.is_empty() && next != Finished {
            result.literal_array_unknown = true;
        }
        if let Some(completed) = completed {
            if result.literal_array_route.replace(completed).is_some_and(|old| old != completed) {
                result.literal_array_unknown = true;
            }
        }
        state.literal_array_phase = next;
    }

    pub(super) fn visit(
        &self,
        mode: AggregateObservationMode,
        point: &ReferenceFlowPoint,
        state: &mut AggregateEdgeState,
        result: &mut AggregateFlowResult,
        sites: &FunctionCallSites,
        value_deadlines: Option<&[usize]>,
    ) {
        if mode.literal {
            self.visit_literal_array(point, state, result);
        }
        if !mode.legacy {
            return;
        }
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
                        state.array_last_recommit = Some(*rhs_point);
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
                            last_producer: state.array_tail_dependency.or(state.array_last_recommit),
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
                            if matches!(value, Some(Value::Aggregate { backing: OwnerSource::Call(producer), field: None }) if producer == rhs) {
                                result.direct_receipt_bindings.entry(rhs).or_insert_with(|| (id, binding.clone()));
                            }
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
                                Some(Value::Aggregate { backing: OwnerSource::Formal(slot), field: None })
                                    if self.formals.len() == 1 && self.raw_formals.is_empty()
                                        && self.formals.get(&slot) == Some(owner)
                                        && self.conditional_formal_moves.contains_key(&id) =>
                                {
                                    state.input_moves.insert(id, InputOrigin::ConditionalFormal(slot));
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
                CfgPointKind::Call { output: 1, .. } => self.secondary_owner_outputs.get(&id)
                    .map(|&producer| Value::Aggregate { backing: OwnerSource::Call(producer), field: None }),
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
                                        } else if let InputOrigin::ConditionalFormal(formal) = owner {
                                            self.conditional_formal_moves.get(&moved) == Some(&id)
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
                                if let (Some(InputOrigin::ConditionalFormal(formal)), ArgumentProvenance::WholeMove(moved)) = (source, moved) {
                                    let transfer = (formal, id, ordinal.0, slot, moved);
                                    if result.conditional_formal_forward.is_some_and(|old| old != transfer) {
                                        result.formal_unsupported.insert(formal);
                                    }
                                    result.conditional_formal_forward = Some(transfer);
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
                    offset.or_else(|| (self.calls.contains_key(&id) && !self.secondary_owner_calls.contains(&id)).then_some(Value::Aggregate {
                        backing: OwnerSource::Call(id),
                        field: None,
                    }))
                }
                CfgPointKind::ConditionalOutput {
                    then_point,
                    else_point,
                } => {
                    let values = (state.values.get(then_point), state.values.get(else_point));
                    if let Some(MixedArmProjection { blocks, .. }) = self.mixed_arm_projection.filter(|shape| shape.owner_projection == id) {
                        match values {
                            (Some(value), None) | (None, Some(value)) => {
                                let arm = usize::from(values.0.is_none());
                                let backing = match value {
                                    Value::Aggregate { backing: OwnerSource::Allocation(point), .. }
                                        if !state.moved.contains(&OwnerSource::Allocation(*point))
                                            && self.allocation_slots.get(point).is_some_and(|(block, _)| *block == blocks[arm].0)
                                        => Some(*point),
                                    _ => None,
                                };
                                result.mixed_arm_backings[arm].observe(backing);
                            }
                            _ => {
                                for arm in &mut result.mixed_arm_backings {
                                    arm.observe(None);
                                }
                            }
                        }
                    }
                    match values {
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
                    }
                }
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
    MixedBranch,
    Allocation { block: usize, item: usize },
    Forward { call: OccurrenceOrdinal },
    Recommit { call: usize, output: usize },
    Structured { output: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClosedCall {
    Receive,
    Forward,
    ResultArm,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClosedInput {
    Transfer(usize),
    ForwardFormal(usize),
    ForwardFormalDeferred(usize),
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ClosedLiteralArrayUnit {
    pub(crate) expression: usize,
    pub(crate) writer_block: usize,
    pub(crate) writer_side: ArrayWriterSide,
    pub(crate) retained_block: Option<usize>,
    pub(crate) other_writer: Option<ClosedLiteralArrayWrite>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ClosedLiteralArrayWrite {
    pub(crate) call: usize,
    pub(crate) destination: usize,
    pub(crate) extent: ReplacementArrayExtent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClosedLiteralArray {
    FreshLeaf {
        receipt: OriginalArrayReceipt,
        length_occurrence: usize,
        extent: FreshArrayExtent,
        output: usize,
    },
    Replacement {
        original: OriginalArrayReceipt,
        original_length_occurrence: usize,
        original_elements: u64,
        call: usize,
        destination: usize,
        exit: CfgPointId,
        block: usize,
        extent: ReplacementArrayExtent,
        unit: Option<ClosedLiteralArrayUnit>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ClosedFreshRecommit {
    pub(crate) destination: usize,
    pub(crate) size: RecommittedArrayExtent,
    pub(crate) argument: Option<usize>,
    pub(crate) forward: bool,
}

#[derive(Debug)]
pub(crate) struct ClosedOwnerRelease {
    pub(crate) receipt: usize,
    pub(crate) guard: Option<usize>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ReceiptGate {
    #[default]
    Retained,
    Transferred,
    Guard(usize),
}

#[derive(Debug, Default)]
pub(crate) struct ClosedConditionalInputs {
    pub(crate) guards: Vec<(usize, [ReceiptGate; 2])>,
    pub(crate) consumed_on_both_arms: Vec<usize>,
}

#[derive(Debug, Default)]
pub(crate) struct ClosedBody {
    pub(crate) scalar_descriptor_assignment: Option<(usize, usize, usize)>,
    pub(crate) checked_backing_producer: Option<ClosedCheckedBacking>,
    pub(crate) checked_backing_wrapper: Option<usize>,
    pub(crate) checked_backing_consumer: Option<(usize, usize)>,
    pub(crate) checked_backing_optional: bool,
    pub(crate) returning: Option<ClosedReturn>,
    mixed_branch_return: Option<Box<MixedBranchReturn>>,
    pub(crate) named_result_arms: BTreeMap<usize, (usize, usize)>, // block -> (output, producer)
    pub(crate) ordinary_result: bool,
    pub(crate) ordinary_calls: BTreeMap<usize, SourceSpan>,
    pub(crate) calls: BTreeMap<usize, ClosedCall>,
    pub(crate) recommits: BTreeMap<usize, usize>,
    pub(crate) identity_relay: bool,
    pub(crate) scope_exits: BTreeMap<usize, CfgPointId>,
    pub(crate) releases: BTreeMap<CfgPointId, Vec<ClosedOwnerRelease>>,
    pub(crate) conditional_inputs: BTreeMap<usize, ClosedConditionalInputs>,
    pub(crate) conditional_guard_count: usize,
    pub(crate) deferred_call_inputs: BTreeMap<usize, usize>,
    pub(crate) input: Option<usize>,
    pub(crate) input_mixed: bool,
    // Checked primary call occurrence -> exact destination place occurrence.
    pub(crate) same_array_rebinds: BTreeMap<usize, usize>,
    pub(crate) fresh_array_recommits: BTreeMap<usize, ClosedFreshRecommit>,
    pub(crate) local_array_failure: Option<ClosedLocalArrayRelease>,
    pub(crate) literal_array: Option<ClosedLiteralArray>,
    pub(crate) input_release: Option<CfgPointId>,
    pub(crate) input_release_guard: Option<usize>,
    pub(crate) input_calls: BTreeMap<usize, (usize, ClosedInput)>,
}

impl ClosedBody {
    pub(crate) fn mixed_branch_conditional(&self) -> Option<usize> {
        self.mixed_branch_return.as_ref().map(|plan| plan.shape.conditional.0)
    }

    pub(crate) fn mixed_branch_owner_lane(&self) -> Option<usize> {
        self.mixed_branch_return.as_ref().map(|plan| plan.shape.owner_lane)
    }

    pub(crate) fn mixed_branch_arm(&self, block: usize) -> Option<(usize, usize)> {
        self.mixed_branch_return.as_ref()?.allocations.iter().enumerate()
            .find_map(|(arm, allocation)| (allocation.block.0 == block).then_some((arm, allocation.item)))
    }
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

struct StructuredOwnerReturn {
    named: BTreeMap<CfgPointId, (BlockOrdinal, OccurrenceOrdinal)>,
    output: OccurrenceOrdinal,
    leaves: BTreeSet<CfgPointId>,
    complete: bool,
}

struct Body<'a> {
    facts: Option<&'a AggregateFlowResult>,
    cfg: &'a ScalarReferenceCfg,
    sites: &'a FunctionCallSites,
    source: &'a SourceIdentity,
    is_monomorphic: bool,
    supports_input_result: bool,
}

impl Body<'_> {
    fn mixed_result_lanes(&self) -> Option<MixedOwnerResultLanes> {
        let [RootOutputType { output: 0, ty: first }, RootOutputType { output: 1, ty: second }] =
            self.sites.boundary.root_outputs.as_slice() else { return None };
        mixed_owner_result_lanes(first, second)
    }

    fn has_mixed_struct_result(&self) -> bool {
        self.mixed_result_lanes().is_some()
    }

    fn structured_owner_return(&self) -> Option<StructuredOwnerReturn> {
        let facts = self.facts?;
        let [ty] = self.sites.boundary.root_outputs.as_slice() else { return None };
        if !matches!(ty.ty, ScalarType::Struct(_)) { return None; }
        let structure = &self.sites.structure;
        let output_at = |block: BlockOrdinal| {
            let mut outputs = structure.blocks[block.0].final_outputs();
            let output = outputs.next()?;
            (output.output == 0 && outputs.next().is_none()).then_some(output.expression)
        };
        let output = output_at(structure.root)?;
        if !matches!(self.sites.conditionals.get(&output), Some(ConditionalRegion::Value { .. })) {
            return None;
        }
        let mut result = StructuredOwnerReturn { named: BTreeMap::new(), output, leaves: BTreeSet::new(), complete: true };
        let calls: BTreeMap<_, _> = facts.calls.iter().map(|(&point, &ordinal)| (ordinal.0, point)).collect();
        let mut pending = vec![(structure.root, output, 0)];
        while let Some((block, expression, depth)) = pending.pop() {
            if depth > MAX_RECEIPT_FORK_DEPTH { result.complete = false; }
            if let Some(ConditionalRegion::Value { then_block, else_block, .. }) = self.sites.conditionals.get(&expression) {
                for block in [*else_block, *then_block] {
                    if let Some(child) = output_at(block) {
                        pending.push((block, child, depth + 1));
                    } else {
                        result.complete = false;
                    }
                }
            } else if let Some(&call) = calls.get(&expression.0) {
                if !result.leaves.insert(call) { result.complete = false; }
            } else if let Some(producer) = self.named_terminal_producer(block, expression, &calls, &ty.ty) {
                if !result.leaves.insert(producer) { result.complete = false; }
                result.named.insert(producer, (block, expression));
            } else {
                result.complete = false;
            }
        }
        Some(result)
    }

    // The immediately preceding statement must install exactly the owning call
    // whose unprojected lexical value this terminal consumes. A matching name or
    // type alone is not evidence of backing identity.
    fn named_terminal_producer(
        &self, block: BlockOrdinal, terminal: OccurrenceOrdinal, calls: &BTreeMap<usize, CfgPointId>,
        ty: &ScalarType,
    ) -> Option<CfgPointId> {
        let [.., prior, last] = self.sites.structure.blocks.get(block.0)?.items.as_slice() else { return None };
        let BlockItemKind::FinalOutput(output) = &last.kind else { return None };
        let BlockItemKind::Binding(bind) = &prior.kind else { return None };
        let BindingInitialization::Outputs(init) = &bind.initialization else { return None };
        let [receiver] = bind.receivers() else { return None };
        let PlainNameIdentity::Lexical(identity) = self.sites.prepared_plain_name(terminal)? else { return None };
        let producer = *calls.get(&init.primary.0)?;
        let (bound, actual) = self.facts?.direct_receipt_bindings.get(&producer)?;
        let points = &self.cfg.points;
        let bind_point = points.get(bound.0)?;
        let [read] = bind_point.successors.as_slice() else { return None };
        let read = points.get(read.0)?;
        let [moved] = read.successors.as_slice() else { return None };
        let moved = points.get(moved.0)?;
        let span = self.sites.occurrences.get(terminal.0)?.span;
        (output.expression == terminal && init.secondary.is_empty() && &receiver.ty == ty
            && actual == identity && identity.declaration_span == receiver.span
            && points.get(producer.0)?.scope == bind_point.scope
            && bind_point.scope == read.scope && read.scope == moved.scope
            && read.source_span.range == span && moved.source_span.range == span
            && matches!(&read.kind, CfgPointKind::Read { binding } if binding == identity)
            && matches!(&moved.kind, CfgPointKind::Move { owner, place, ty: moved_ty }
                if owner == identity && place.binding == *identity && place.projections.is_empty() && moved_ty == ty))
            .then_some(producer)
    }

    fn recommitted_root_output(&self, producer: CfgPointId) -> Option<(OccurrenceOrdinal, OccurrenceOrdinal)> {
        let facts = self.facts?;
        let [output] = self.sites.boundary.root_outputs.as_slice() else { return None };
        if output.output != 0 || !matches!(output.ty, ScalarType::Struct(_)) || facts.returns.len() != 1 {
            return None;
        }
        let (&returned, _) = facts.returns.first_key_value()?;
        let (bound, binding) = facts.direct_receipt_bindings.get(&producer)?;
        let root = self.cfg.points.first()?.scope;
        let point = self.cfg.points.get(returned.0)?;
        let CfgPointKind::ReturnOutput { output: 0, value_point, checked: false } = point.kind else { return None };
        let value = self.cfg.points.get(value_point.0)?;
        if point.scope != root || value.scope != root || *bound >= value_point || value_point >= returned
            || !matches!(&value.kind, CfgPointKind::Read { binding: read } if read == binding)
        {
            return None;
        }
        let mut outputs = self.sites.structure.blocks[self.sites.structure.root.0].final_outputs();
        let output = outputs.next()?;
        if output.output != 0 || outputs.next().is_some() { return None; }
        Some((*facts.calls.get(&producer)?, output.expression))
    }

    fn is_exact_identity_relay(&self) -> bool {
        let Some(facts) = self.facts else { return false };
        let ([parameter], [output]) = (
            self.sites.boundary.parameters.as_slice(),
            self.sites.boundary.root_outputs.as_slice(),
        ) else { return false };
        let Some(formal) = facts.formals.get(&0) else { return false };
        if !self.is_monomorphic || parameter.ordinal != 0 || output.output != 0
            || !matches!(parameter.ty, ScalarType::Struct(_)) || parameter.ty != output.ty
            || !self.cfg.allocations.is_empty()
            || !facts.unsupported.is_empty() || facts.raw_invalid.is_some()
            || !self.return_owners().eq([Some(OwnerSource::Formal(0))])
        {
            return false;
        }
        let mut read = None;
        let mut moved = false;
        let mut returned = false;
        for (i, point) in self.cfg.points.iter().enumerate() {
            if self.cfg.points.get(i + 1).map_or(!point.successors.is_empty(), |next|
                point.successors.as_slice() != [next.id])
            {
                return false;
            }
            match &point.kind {
                CfgPointKind::Entry | CfgPointKind::ScopeEnter | CfgPointKind::ScopeExit | CfgPointKind::Return => {}
                CfgPointKind::Bind { binding } if binding == formal => {}
                CfgPointKind::Read { binding } if binding == formal && read.is_none() => read = Some(point.id),
                CfgPointKind::Move { place, owner, ty } if read.is_some() && !moved
                    && owner == formal && place.binding == *formal && place.projections.is_empty()
                    && *ty == parameter.ty => moved = true,
                CfgPointKind::ReturnOutput { output: 0, value_point, checked: false }
                    if moved && !returned && read == Some(*value_point) => returned = true,
                _ => return false,
            }
        }
        returned
    }

    fn return_owners(&self) -> impl Iterator<Item = Option<OwnerSource>> + '_ {
        let lane = self.mixed_result_lanes().map(|lanes| lanes.owner);
        self.facts.into_iter().flat_map(move |facts| facts.return_owners(self.cfg, lane))
    }
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

#[derive(Clone, Copy)]
struct LiteralArrayPair {
    caller: BodyKey,
    callee: BodyKey,
    established_alternate: Option<(BodyKey, LiteralArrayBind, OccurrenceOrdinal)>,
    original: LiteralArrayBind,
    fresh: LiteralArrayBind,
    final_output: OccurrenceOrdinal,
    write: LiteralArrayWrite,
    exit: CfgPointId,
    block: BlockOrdinal,
    unit: Option<LiteralArrayUnit>,
}

#[derive(Clone, Copy, Default)]
struct LiteralArrayParameters<'a> {
    caller: &'a [ParameterSlot],
    callee: &'a [ParameterSlot],
    signature: &'a [ScalarType],
}

struct LiteralArrayEnrollment {
    caller: BodyKey,
    exit: CfgPointId,
    block: BlockOrdinal,
    replacement: ClosedLiteralArray,
    new_leaf: Option<(BodyKey, ClosedLiteralArray)>,
}

fn preflight_literal_array_pair(
    result: &ClosedAggregates,
    pair: LiteralArrayPair,
    parameters: LiteralArrayParameters<'_>,
) -> Option<LiteralArrayEnrollment> {
    let LiteralArrayPair { caller, callee, established_alternate, original, fresh, final_output, write, exit, block, unit } = pair;
    if caller == callee || result.bodies.contains_key(&caller) || write.rhs == write.writer {
        return None;
    }
    let same_parameters = match (write.argument, parameters.caller, parameters.callee, parameters.signature) {
        (None, [], [], []) => fresh.extent != FreshArrayExtent::Formal,
        (Some(_), [caller], [callee], [actual]) =>
            caller.ordinal == 0 && callee.ordinal == 0 && ordinary_value(actual)
                && &caller.ty == actual && &callee.ty == actual
                && (fresh.extent != FreshArrayExtent::Formal || *actual == ScalarType::U64),
        _ => false,
    };
    if !same_parameters {
        return None;
    }
    let FreshArrayExtent::Literal(original_elements) = original.extent else { return None };
    let extent_for = |extent: FreshArrayExtent, write: LiteralArrayWrite| match extent {
        FreshArrayExtent::Literal(elements) => Some(ReplacementArrayExtent::Literal(elements)),
        FreshArrayExtent::Formal => write.argument.map(|argument|
            ReplacementArrayExtent::Argument { occurrence: argument.occurrence.0 }),
    };
    let replacement_extent = extent_for(fresh.extent, write)?;
    let other_writer = if let Some(other) = unit.and_then(|unit| unit.other_write) {
        if other.rhs == other.writer || other.rhs == write.rhs || other.writer == write.writer
            || other.producer == write.producer || other.destination == write.destination
            || other.argument.is_some() != write.argument.is_some()
            || write.argument.zip(other.argument).is_some_and(|(a, b)|
                a.read == b.read || a.occurrence == b.occurrence)
            || !unit.is_some_and(|unit| unit.writer_side == ArrayWriterSide::Then && unit.retained.is_some())
        {
            return None;
        }
        let extent = established_alternate.map_or(fresh.extent, |(_, allocation, _)| allocation.extent);
        let extent = extent_for(extent, other)?;
        Some(ClosedLiteralArrayWrite { call: other.producer.0, destination: other.destination.0, extent })
    } else {
        None
    };
    let to_receipt = |allocation: LiteralArrayBind| OriginalArrayReceipt {
        origin: allocation.bind,
        allocation: (allocation.position.block.0, allocation.position.item),
    };
    let leaf_plan = |allocation: LiteralArrayBind, output: OccurrenceOrdinal| ClosedLiteralArray::FreshLeaf {
        receipt: to_receipt(allocation), length_occurrence: allocation.length_occurrence.0,
        extent: allocation.extent, output: output.0,
    };
    let producer = leaf_plan(fresh, final_output);
    let existing_primary = result.bodies.get(&callee);
    let new_leaf = if let Some((alternate, allocation, output)) = established_alternate {
        if caller == alternate || callee == alternate || other_writer.is_none()
            || !matches!(fresh.extent, FreshArrayExtent::Literal(_))
            || !matches!(allocation.extent, FreshArrayExtent::Literal(_))
            || existing_primary.and_then(|plan| plan.literal_array) != Some(producer)
            || result.bodies.get(&alternate).and_then(|plan| plan.literal_array) != Some(leaf_plan(allocation, output))
        {
            return None;
        }
        None
    } else {
        match existing_primary {
            Some(existing) if existing.literal_array == Some(producer) => None,
            Some(_) => return None,
            None => Some((callee, producer)),
        }
    };
    let replacement = ClosedLiteralArray::Replacement {
        original: to_receipt(original), original_length_occurrence: original.length_occurrence.0,
        original_elements,
        call: write.producer.0, destination: write.destination.0, exit, block: block.0,
        extent: replacement_extent,
        unit: unit.map(|unit| ClosedLiteralArrayUnit {
            expression: unit.expression.0, writer_block: unit.writer_block.0, writer_side: unit.writer_side,
            retained_block: unit.retained.map(|retained| retained.block.0),
            other_writer,
        }),
    };
    Some(LiteralArrayEnrollment { caller, exit, block, replacement, new_leaf })
}

fn enroll_literal_array_pair(
    result: &mut ClosedAggregates,
    pair: LiteralArrayPair,
    parameters: LiteralArrayParameters<'_>,
) {
    // No fallible checks or intervening mutations after preflight: commit both
    // body plans and the sole scope cleanup together.
    let Some(LiteralArrayEnrollment { caller, exit, block, replacement, new_leaf }) =
        preflight_literal_array_pair(result, pair, parameters)
    else { return };
    if let Some((callee, producer)) = new_leaf {
        result.bodies.entry(callee).or_default().literal_array = Some(producer);
    }
    let plan = result.bodies.entry(caller).or_default();
    plan.scope_exits.insert(block.0, exit);
    plan.literal_array = Some(replacement);
}

fn install_literal_array_plans(
    callers: &[(BodyKey, &Body<'_>, &LiteralArrayRoute)],
    bodies: &BTreeMap<BodyKey, Body<'_>>,
    declarations: &HashMap<(&SourceIdentity, ByteSpan), (usize, usize)>,
    result: &mut ClosedAggregates,
) {
    let resolve = |callee: BodyKey| {
        let leaf = bodies.get(&callee)?;
        if !leaf.is_monomorphic || callee.2.is_some() {
            return None;
        }
        let LiteralArrayRoute::FreshLeaf { allocation, output } =
            leaf.facts?.proved_literal_array_route()? else { return None };
        let [returned] = leaf.sites.boundary.results.as_slice() else { return None };
        Some((allocation, output, &returned.ty, leaf.sites.boundary.parameters.as_slice()))
    };
    let mut distinct = Vec::new();
    for &(caller, body, route) in callers {
        let LiteralArrayRoute::Replacement { original, write, exit, block, unit } = *route else { continue };
        let Some(callee) = target_body(declarations, body.cfg, write.rhs) else { continue };
        let Some((fresh, final_output, returned, formals)) = resolve(callee) else { continue };
        let Some(signature) = body.sites.checked_call_signatures.get(&write.producer) else { continue };
        let [output] = signature.outputs.outputs.as_slice() else { continue };
        if !same_runtime_array_type(&output.ty, returned) {
            continue;
        }
        let mut pair = LiteralArrayPair {
            caller, callee, established_alternate: None, original, fresh, final_output, write, exit, block, unit,
        };
        let parameters = LiteralArrayParameters {
            caller: &body.sites.boundary.parameters, callee: formals, signature: &signature.parameters,
        };
        if let Some(other) = unit.and_then(|unit| unit.other_write) {
            let Some(other_signature) = body.sites.checked_call_signatures.get(&other.producer) else { continue };
            let [other_output] = other_signature.outputs.outputs.as_slice() else { continue };
            if other_signature.parameters != signature.parameters
                || !same_runtime_array_type(&other_output.ty, &output.ty)
            {
                continue;
            }
            let Some(other_callee) = target_body(declarations, body.cfg, other.rhs) else { continue };
            if other_callee != callee {
                let Some((allocation, output, other_returned, other_formals)) =
                    resolve(other_callee) else { continue };
                if !same_runtime_array_type(&other_output.ty, other_returned)
                    || !other_formals.iter().map(|slot| (slot.ordinal, &slot.ty))
                        .eq(formals.iter().map(|slot| (slot.ordinal, &slot.ty)))
                    || !matches!(fresh.extent, FreshArrayExtent::Literal(_))
                    || !matches!(allocation.extent, FreshArrayExtent::Literal(_))
                {
                    continue;
                }
                pair.established_alternate = Some((other_callee, allocation, output));
                distinct.push((pair, parameters));
                continue;
            }
        }
        enroll_literal_array_pair(result, pair, parameters);
    }
    // Existing candidates have first claim; distinct routes may reuse only
    // their exact established producer plans and never write producer plans.
    for (pair, parameters) in distinct {
        enroll_literal_array_pair(result, pair, parameters);
    }
}

fn recommit_extent_occurrence(
    body: &Body<'_>, point: CfgPointId, ordinal: OccurrenceOrdinal, position: usize,
) -> Option<OccurrenceOrdinal> {
    let CfgPointKind::Call { output: 0, argument_points, .. } = &body.cfg.initial_points.get(point.0)?.kind
    else { return None };
    let OccurrenceNode::Expression(argument) = *body.sites.structure.edges.get(&(
        OccurrenceParent::Expression(ordinal), OccurrenceEdge::CallArgument(position),
    ))? else { return None };
    (body.sites.structure.checked_types.expressions.get(&argument) == Some(&ScalarType::U64)
        && argument_points.get(position).is_some()).then_some(argument)
}

fn formal_recommit_extent(
    body: &Body<'_>, point: CfgPointId, ordinal: OccurrenceOrdinal, extent: RecommittedArrayExtent,
) -> Option<RecommittedArrayExtent> {
    let RecommittedArrayExtent::Formal(position) = extent else { return Some(extent) };
    let argument = recommit_extent_occurrence(body, point, ordinal, position)?;
    let PlainNameIdentity::Lexical(identity) = body.sites.prepared_plain_name(argument)? else { return None };
    let CfgPointKind::Call { argument_points, .. } = &body.cfg.initial_points.get(point.0)?.kind
    else { return None };
    let CfgPointKind::Read { binding } = &body.cfg.initial_points.get(argument_points[position].0)?.kind
    else { return None };
    let formal = body.sites.boundary.parameters.iter().find(|slot| slot.span == identity.declaration_span)?;
    (binding == identity && identity.source == *body.source && formal.ty == ScalarType::U64
        && identity.function_span == body.cfg.initial_points.first()?.source_span.range)
        .then_some(RecommittedArrayExtent::Formal(formal.ordinal))
}

fn fresh_recursive_recommit(
    body: &Body<'_>, key: BodyKey, bodies: &BTreeMap<BodyKey, Body<'_>>,
    declarations: &HashMap<(&SourceIdentity, ByteSpan), (usize, usize)>,
) -> Option<RecommittedArrayExtent> {
    let facts = body.facts?;
    if !body.is_monomorphic || !facts.array_neutral || facts.array_invalid
        || facts.array_conditional_forwarded
    {
        return None;
    }
    let mut ground = None;
    let mut recursive = None;
    for path in &facts.array_returns {
        let path = path.as_ref()?;
        let point = path.last_producer?;
        if path.tail_call.is_some() || path.dependencies.len() != 1 || !path.dependencies.contains(&point) {
            return None;
        }
        let (ordinal, _, _) = facts.array_calls.get(&point)?;
        let target = target_body(declarations, body.cfg, point)?;
        if target == key {
            // The ordinary branch-only forwarding slice has one exact self call.
            if recursive.replace((point, *ordinal)).is_some() {
                return None;
            }
        } else {
            let leaf = bodies.get(&target)?;
            if !leaf.is_monomorphic || leaf.source != body.source {
                return None;
            }
            let extent = formal_recommit_extent(body, point, *ordinal, leaf.facts?.fresh_recommit_leaf?)?;
            if ground.replace(extent).is_some_and(|prior| prior != extent) {
                return None;
            }
        }
    }
    let extent = ground?;
    if facts.array_rebinds.len() != facts.array_returns.len() || facts.array_calls.len() != facts.array_returns.len()
        || body.cfg.initial_points.iter().any(|point| match &point.kind {
            CfgPointKind::ReadPlace { .. } => true,
            CfgPointKind::Assign { target, .. } => match extent {
                RecommittedArrayExtent::Literal(_) => false,
                RecommittedArrayExtent::Formal(slot) => body.sites.boundary.parameters.get(slot)
                    .is_none_or(|formal| target.binding.declaration_span == formal.span),
            },
            _ => false,
        })
    {
        return None;
    }
    let (point, ordinal) = recursive?;
    (formal_recommit_extent(body, point, ordinal, extent)? == extent).then_some(extent)
}

#[derive(Eq, Hash, PartialEq)]
struct FreshRecursiveCallSignature(Vec<ScalarTypeIdentity>, Vec<ScalarTypeIdentity>);

fn fresh_recursive_call_signature<'a>(
    parameters: impl Iterator<Item = &'a ScalarType>, outputs: impl Iterator<Item = &'a ScalarType>,
) -> FreshRecursiveCallSignature {
    FreshRecursiveCallSignature(parameters.map(scalar_type_identity).collect(), outputs.map(scalar_type_identity).collect())
}

fn could_call_fresh_recursive_array(
    signature: Option<&CheckedCallSignature>, candidates: &HashSet<FreshRecursiveCallSignature>,
) -> bool {
    signature.is_none_or(|sig| neutral_array_signature(&sig.parameters, &sig.outputs)
        && candidates.contains(&fresh_recursive_call_signature(
            sig.parameters.iter(), sig.outputs.outputs.iter().map(|output| &output.ty),
        )))
}

fn install_fresh_recursive_recommits<'a>(
    reservations: &AggregateReservations, module_at: &impl Fn(usize) -> Option<ModuleInput<'a>>,
    bodies: &BTreeMap<BodyKey, Body<'_>>, declarations: &HashMap<(&SourceIdentity, ByteSpan), (usize, usize)>,
    result: &mut ClosedAggregates,
) -> Result<(), String> {
    let mut returns = BTreeMap::new();
    for (&key, body) in bodies {
        if let Some(extent) = fresh_recursive_recommit(body, key, bodies, declarations) {
            returns.insert(key, extent);
        }
    }
    if returns.is_empty() { return Ok(()) }
    let signatures: HashSet<_> = returns.keys().map(|key| {
        let boundary = &bodies[key].sites.boundary;
        fresh_recursive_call_signature(boundary.parameters.iter().map(|slot| &slot.ty),
            boundary.results.iter().map(|slot| &slot.ty))
    }).collect();
    let mut planned = Vec::new();
    for (&key, body) in bodies {
        let Some(facts) = body.facts.filter(|_| body.is_monomorphic) else { continue };
        if facts.array_rebinds.is_empty() { continue }
        let forward = returns.contains_key(&key);
        let local = (!forward).then(|| facts.allocation_slots.keys().filter_map(|bind| {
            match &body.cfg.initial_points.get(bind.0)?.kind {
                CfgPointKind::Bind { binding } => Some(binding),
                _ => None,
            }
        }).collect::<HashSet<_>>());
        for (&assign, &(call, ref binding, destination)) in &facts.array_rebinds {
            let Some(target) = target_body(declarations, body.cfg, call) else { continue };
            let Some(callee) = bodies.get(&target).filter(|callee| callee.source == body.source) else { continue };
            let extent = if forward {
                returns.get(&target).copied().or_else(|| callee.facts?.fresh_recommit_leaf)
            } else {
                if body.cfg.initial_points[assign.0].scope != ReferenceScopeId(0)
                    || !local.as_ref().is_some_and(|owners| owners.contains(binding))
                {
                    continue;
                }
                returns.get(&target).copied()
            };
            let Some(size) = extent else { continue };
            let Some(&(ordinal, _, _)) = facts.array_calls.get(&call) else { continue };
            let argument = match size {
                RecommittedArrayExtent::Literal(_) => None,
                RecommittedArrayExtent::Formal(position) => {
                    let Some(occurrence) = recommit_extent_occurrence(body, call, ordinal, position) else { continue };
                    Some(occurrence.0)
                }
            };
            planned.push((key, target, ordinal, ClosedFreshRecommit {
                destination: destination.0, size, argument, forward,
            }));
        }
    }
    let covered: HashSet<_> = planned.iter().map(|&(caller, target, ordinal, _)| (caller, target, ordinal)).collect();
    let mut definitions = HashMap::<&SourceIdentity, HashMap<ByteSpan, BodyKey>>::new();
    for &key @ (module, item, _) in returns.keys() {
        let input = module_at(module).ok_or("missing fresh array result module")?;
        let Some(ScalarItem::Function(function)) = input.items.get(item) else {
            return Err("missing fresh array result declaration".into());
        };
        definitions.entry(input.source).or_default().insert(function.name_span, key);
    }
    let declaration_key = |source: &SourceIdentity, span: ByteSpan| {
        definitions.get(source).and_then(|functions| functions.get(&span)).copied()
    };
    let mut rejected = BTreeSet::new();
    for &caller @ (module, item, arm) in reservations.bodies.keys() {
        let input = module_at(module).ok_or("missing fresh return use module")?;
        let sites = input.calls.function_body(item, arm).ok_or("missing fresh return use body")?;
        if !matches!(&sites.plain_names, PlainNameFacts::Prepared(_))
            || sites.allocation_length_calls.iter().any(|ordinal| !sites.allocation_length_call_targets.contains_key(ordinal))
        {
            return Err(format!("cannot verify fresh array return without prepared ownership uses in {:?}", input.source));
        }
        for escape in &sites.callable_escapes {
            if let Some(key) = declaration_key(&escape.source, escape.range) {
                rejected.insert(key);
            }
        }
        let direct_result = |target: &ResolvedCallTarget, ordinal: Option<OccurrenceOrdinal>| {
            match target {
                ResolvedCallTarget::ModuleCallable { source, declaration_span, concrete } => {
                    declaration_key(source, *declaration_span)
                        .filter(|key| !matches!(concrete, ConcreteCallSelection::Declaration)
                            || !ordinal.is_some_and(|ordinal| covered.contains(&(caller, *key, ordinal))))
                }
                _ => None,
            }
        };
        for (&ordinal, target) in &sites.allocation_length_call_targets {
            if could_call_fresh_recursive_array(sites.checked_call_signatures.get(&ordinal), &signatures)
                && matches!(target, ResolvedCallTarget::LocalCallable(_) | ResolvedCallTarget::CallableField { .. })
            {
                return Err(format!("cannot verify indirect fresh array use in {:?}", input.source));
            }
            if let Some(key) = direct_result(target, None) {
                rejected.insert(key);
            }
        }
        for call in &sites.call_occurrences {
            let target = &sites.sites.get(call.site).ok_or("missing fresh return call occurrence")?.target;
            if matches!(target, ResolvedCallTarget::LocalCallable(_) | ResolvedCallTarget::CallableField { .. })
                && could_call_fresh_recursive_array(sites.checked_call_signatures.get(&call.ordinal), &signatures)
            {
                return Err(format!("cannot verify indirect fresh array use in {:?}", input.source));
            }
            if let Some(key) = direct_result(target, Some(call.ordinal)) {
                rejected.insert(key);
            }
        }
    }
    for &module in &reservations.modules {
        let input = module_at(module).ok_or("missing fresh return use module")?;
        let ModuleInitializationDetail::Occurrences(topology) = &input.calls.initialization.detail else {
            return Err("missing fresh return initializer use inventory".into());
        };
        for escape in &topology.callable_escapes {
            if let Some(key) = declaration_key(&escape.source, escape.range) {
                rejected.insert(key);
            }
        }
        for &ordinal in &topology.calls {
            match input.calls.initialization.call_target(ordinal) {
                Some(ModuleInitializationCallTarget::Direct { declaration, .. }
                    | ModuleInitializationCallTarget::Generic { declaration, .. }) => {
                    if let Some(key) = declaration_key(&declaration.source, declaration.range) {
                        rejected.insert(key);
                    }
                }
                Some(ModuleInitializationCallTarget::CallableBinding(_) | ModuleInitializationCallTarget::CallableField { .. })
                    if could_call_fresh_recursive_array(topology.checked_call_signatures.get(&ordinal), &signatures) =>
                        return Err(format!("cannot verify indirect fresh array initializer use in {:?}", input.source)),
                None => return Err(format!("missing fresh array initializer call target in {:?}", input.source)),
                _ => {}
            }
        }
    }
    if let Some(key) = rejected.first() {
        return Err(format!("fresh recursive array result has an unsupported ownership receiver in {:?}", bodies[key].source));
    }
    for (key, target, ordinal, receipt) in planned {
        let plan = result.bodies.entry(key).or_default();
        let has_literal_replacement = matches!(plan.literal_array,
            Some(ClosedLiteralArray::Replacement { call, unit, .. })
                if call == ordinal.0 || unit.and_then(|unit| unit.other_writer)
                    .is_some_and(|writer| writer.call == ordinal.0));
        if plan.same_array_rebinds.contains_key(&ordinal.0) || has_literal_replacement {
            return Err(format!("overlapping array ownership receipts at {key:?} for {target:?}"));
        }
        plan.fresh_array_recommits.insert(ordinal.0, receipt);
    }
    Ok(())
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
    let identity_relays: BTreeSet<_> = bodies.iter().filter_map(|(&key, body)|
        (key.2.is_none() && body.is_exact_identity_relay()).then_some(key)
    ).collect();
    // Only observed whole-move origins can re-key backing in the owning
    // caller. Direct identity relays themselves keep ordinary value ABI.
    #[derive(Clone, Copy)]
    struct RecommitOrigin {
        previous: CfgPointId,
        birth: CfgPointId,
    }
    let mut recommit_observers = BTreeMap::<(BodyKey, CfgPointId), BTreeSet<(BodyKey, usize)>>::new();
    let mut recommit_edges = BTreeMap::<(BodyKey, CfgPointId), Vec<(CfgPointId, Option<CfgPointId>)>>::new();
    let mut structured_returns = BTreeMap::new();
    let mut recommit_returns = BTreeSet::new();
    let mut outgoing = BTreeSet::new();
    let mut pending = Vec::new();
    let mut callers = BTreeMap::<BodyKey, Vec<BodyKey>>::new();
    let mut forward_edges = BTreeMap::new();
    let mut conditional_forward_edges = BTreeMap::new();
    let mut modes = BTreeMap::<(BodyKey, usize), (bool, bool)>::new();
    let mut seed_targets = BTreeMap::<BodyKey, BTreeSet<(BodyKey, usize)>>::new();
    let mut input_users = BTreeMap::<(BodyKey, usize), Vec<(BodyKey, CfgPointId, usize, ArgumentProvenance, &BTreeSet<Option<InputOrigin>>) >>::new();
    let mut array_callsites = Vec::new();
    let mut literal_array_callers = Vec::new();
    let mut array_leaves = BTreeSet::new();
    let mut array_neighbors = BTreeMap::<BodyKey, BTreeSet<BodyKey>>::new();
    let mut result = ClosedAggregates::default();
    for (&key, body) in &bodies {
        if let Some(facts) = body.facts {
            // Borrow only completed caller proofs during this existing pass.
            // BTreeMap order preserves enrollment precedence at finalization.
            if let Some(route) = facts.proved_literal_array_caller(key, body.is_monomorphic) {
                literal_array_callers.push((key, body, route));
            }

            if body.is_monomorphic {
                if let Some(proof) = facts.scalar_descriptor_assignment {
                    result.bodies.entry(key).or_default().scalar_descriptor_assignment = Some(proof);
                }
            }
            if body.is_monomorphic && !facts.array_rebinds.is_empty() {
                array_callsites.push((key, body.cfg, facts));
            }
            if body.is_monomorphic && !facts.array_invalid && !facts.array_returns.is_empty() {
                if let Some(mut dependencies) = grounded_array_return_dependencies(
                    key,
                    &facts.array_returns,
                    |point| {
                        let target = target_body(&declarations, body.cfg, point)?;
                        if facts.array_conditional_forwarded && target != key
                            && !bodies.get(&target).and_then(|callee| callee.facts)
                                .is_some_and(|callee| callee.array_neutral)
                        {
                            return None;
                        }
                        Some(target)
                    },
                ) {
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
                    input_users.entry(receiver).or_default().push((key, point, ordinal, moved, origins));
                    let mode = modes.entry(receiver).or_default();
                    for source in origins {
                        match *source {
                            Some(InputOrigin::Owner(OwnerSource::Call(producer)) | InputOrigin::Immediate(producer)) => {
                                if identity_relays.contains(&target) {
                                    let whole_move = match (*source, moved) {
                                        (Some(InputOrigin::Owner(_)), ArgumentProvenance::WholeMove(moved))
                                            if origins.len() == 1 && slot == 0 => Some(moved),
                                        _ => None,
                                    };
                                    recommit_edges.entry((key, producer)).or_default().push((point, whole_move));
                                } else if let Some(origin) = target_body(&declarations, body.cfg, producer) {
                                    if identity_relays.contains(&origin) {
                                        recommit_observers.entry((key, producer)).or_default().insert(receiver);
                                    } else {
                                        seed_targets.entry(origin).or_default().insert(receiver);
                                    }
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
                    if let Some((formal, call, forward_ordinal, forward_slot, _)) = facts.conditional_formal_forward {
                        if (point, ordinal, slot) == (call, forward_ordinal, forward_slot) {
                            conditional_forward_edges.insert((key, formal), (target, slot));
                        }
                    }
                }
            }
        }
        if let Some(shape) = body.structured_owner_return().filter(|shape| !shape.leaves.is_empty()) {
            for &leaf in &shape.leaves {
                if let Some(target) = target_body(&declarations, body.cfg, leaf) {
                    callers.entry(target).or_default().push(key);
                }
            }
            structured_returns.insert(key, shape);
        }
        for source in body.return_owners().flatten() {
            match source {
                OwnerSource::Formal(_) => {}
                OwnerSource::Allocation(_) => {
                    if outgoing.insert(key) {
                        pending.push(key);
                    }
                }
                OwnerSource::Call(point) => {
                    if let Some(target) = target_body(&declarations, body.cfg, point) {
                        if identity_relays.contains(&target) {
                            recommit_returns.insert((key, point));
                        } else {
                            callers.entry(target).or_default().push(key);
                        }
                    }
                }
            }
        }
    }
    let mut recommitted = BTreeMap::<(BodyKey, CfgPointId), RecommitOrigin>::new();
    let mut recommit_waiting = BTreeMap::<BodyKey, Vec<_>>::new();
    for &(caller, producer) in recommit_edges.keys() {
        if let Some(target) = target_body(&declarations, bodies[&caller].cfg, producer) {
            recommit_waiting.entry(target).or_default().push((caller, producer));
        }
    }
    let mut recommit_pending = Vec::new();
    // Only independently allocated owners seed the two worklists. A wrapper
    // becomes an owning producer when its returned recommit has such a seed.
    // Each observed call edge is consumed once, including through wrapper chains.
    loop {
        if let Some(target) = pending.pop() {
            if let Some(body_callers) = callers.remove(&target) {
                for caller in body_callers {
                    if outgoing.insert(caller) {
                        pending.push(caller);
                    }
                }
            }
            if let Some(sites) = recommit_waiting.remove(&target) {
                recommit_pending.extend(sites);
            }
            continue;
        }
        let Some(node @ (caller, producer)) = recommit_pending.pop() else { break };
        let Some(edges) = recommit_edges.remove(&node) else { continue };
        let body = &bodies[&caller];
        let facts = body.facts.expect("recommit input required caller facts");
        let unsupported = || format!("aggregate recommit has no exact local whole-owner route in {:?}", body.source);
        let [(consumer, Some(moved))] = edges.as_slice() else { return Err(unsupported()); };
        let (consumer, moved) = (*consumer, *moved);
        let points = &body.cfg.points;
        let (original_bind, binding) = facts.direct_receipt_bindings.get(&producer).ok_or_else(unsupported)?;
        let (result_bind, _) = facts.direct_receipt_bindings.get(&consumer).ok_or_else(unsupported)?;
        let birth = recommitted.get(&node).map_or(producer, |origin| origin.birth);
        let root = points[0].scope;
        if !body.is_monomorphic || producer >= *original_bind || *original_bind >= moved || moved >= consumer
            || consumer >= *result_bind
            || [producer, *original_bind, moved, consumer, *result_bind].iter().any(|point| points[point.0].scope != root)
            || !matches!(&points[moved.0].kind, CfgPointKind::Move { owner, place, .. }
                if owner == binding && place.binding == *binding && place.projections.is_empty())
            || facts.conditional_input_barriers.range(producer..=*result_bind).next().is_some()
            || !facts.calls.contains_key(&producer) || !facts.calls.contains_key(&consumer)
            || recommitted.insert((caller, consumer), RecommitOrigin { previous: producer, birth }).is_some()
        {
            return Err(unsupported());
        }
        // This relay must return the caller's descriptor, not the ordinary
        // parameter copy in a callee-owned stack slot.
        let relay = target_body(&declarations, body.cfg, consumer).filter(|relay| identity_relays.contains(relay))
            .ok_or_else(unsupported)?;
        result.bodies.entry(relay).or_default().identity_relay = true;
        let recommitted_value = (caller, consumer);
        if recommit_returns.contains(&recommitted_value) && outgoing.insert(caller) {
            pending.push(caller);
        }
        recommit_pending.push(recommitted_value);
        if let Some(observers) = recommit_observers.remove(&recommitted_value) {
            for receiver in observers {
                modes.get_mut(&receiver).expect("indexed recommit receiver").0 = true;
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
    let finalize_cohorts = |mut result: ClosedAggregates| -> Result<ClosedAggregates, String> {
        install_ordinary(&mut result);
        install_literal_array_plans(&literal_array_callers, &bodies, &declarations, &mut result);
        install_fresh_recursive_recommits(reservations, &module_at, &bodies, &declarations, &mut result)?;
        Ok(result)
    };
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
        return finalize_cohorts(result);
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
            if outgoing.contains(&body) && bodies.get(&body).is_some_and(Body::has_mixed_struct_result) {
                return Err(format!("mixed aggregate result requires a local binding in {:?}", region.source));
            }
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
    // A single direct transfer needs no join. Up to two terminal transfers may
    // defer receipt retirement, but only an exact opposite-arm pair is valid.
    #[derive(Clone, Copy)]
    struct TerminalInputSite {
        ordinal: usize,
        consumer: CfgPointId,
        moved: CfgPointId,
    }
    let mut discharged_by_inputs = BTreeMap::<(BodyKey, CfgPointId), [Option<TerminalInputSite>; 2]>::new();
    let active = |caller, source| {
        let producer = match source {
            Some(InputOrigin::Owner(OwnerSource::Call(producer)) | InputOrigin::Immediate(producer)) => producer,
            _ => return None,
        };
        (recommitted.contains_key(&(caller, producer))
            || target_body(&declarations, bodies[&caller].cfg, producer)
                .is_some_and(|target| outgoing.contains(&target))).then_some(producer)
    };
    let mut demand: Vec<_> = modes.iter().filter_map(|(&formal, mode)| mode.0.then_some(formal)).collect();
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
        let Some(next) = forward_edges.get(&formal).or_else(|| conditional_forward_edges.get(&formal)) else { continue };
        let source = modes[&formal];
        let dest = modes.get_mut(next).ok_or_else(|| "missing forwarded formal input use".to_owned())?;
        let combined = (dest.0 || source.0, dest.1 || source.1);
        if *dest != combined {
            *dest = combined;
            demand.push(*next);
        }
    }
    // A body with a conditional formal may also close ordinary local receipts.
    // Hand off its already-owned certificate rather than walking its CFG again.
    for (&(caller, _), origin) in &recommitted {
        if discharged_by_inputs.insert((caller, origin.previous), [None; 2]).is_some() {
            return Err(format!("aggregate recommit has competing ownership transfers in {:?}", bodies[&caller].source));
        }
    }
    let mut conditional_input_certificates = BTreeMap::new();
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
            if conditional_forward_edges.contains_key(&(callee, slot)) || exits.next().is_some() {
                return Err(format!("aggregate owning formal has both release and transfer in {:?} at slot {slot}", body.source));
            }
            forward_predecessors.entry(next).or_default().push((callee, slot));
            None
        } else if let Some(&next) = conditional_forward_edges.get(&(callee, slot)) {
            let unsupported = || format!(
                "aggregate owning formal conditional transfer has no exact release route in {:?} at slot {slot}",
                body.source,
            );
            let Some((endpoint @ (exit, _), false)) = exits.next() else { return Err(unsupported()); };
            if exits.next().is_some() || mixed || modes[&next].1
                || forward_edges.contains_key(&next) || conditional_forward_edges.contains_key(&next)
                || !facts.raw_uses.is_empty() || !ordinary_formal_outputs(&body.sites.boundary.root_outputs)
                || !facts.endpoints.get(&endpoint).is_some_and(|endpoint| {
                    endpoint.has_conditional_unique_receipt(OwnerSource::Formal(slot))
                        && endpoint.possible.get(&OwnerSource::Formal(slot)).and_then(Option::as_ref) == facts.formals.get(&slot)
                })
            {
                return Err(unsupported());
            }
            let Some((_, consumer, _, _, moved)) = facts.conditional_formal_forward else { return Err(unsupported()); };
            let points = &body.cfg.points;
            let certificate = ConditionalInputCertificate::build(
                points, &facts.conditional_input_regions, &facts.conditional_input_barriers,
            );
            let (fork, side, boundary) = certificate.consuming_arm(points, points[0].id, consumer, Some(exit))
                .ok_or_else(unsupported)?;
            if fork != boundary || certificate.paths[moved.0] != certificate.paths[consumer.0]
                || !matches!(points[exit.0].kind, CfgPointKind::ScopeExit)
            {
                return Err(unsupported());
            }
            let plan = result.bodies.entry(callee).or_default();
            let expression = certificate.forks[fork].region.expression.0;
            let mut arms = [ReceiptGate::Retained; 2];
            arms[side] = ReceiptGate::Transferred;
            let guard = plan.conditional_guard_count;
            plan.conditional_guard_count += 1;
            plan.input_release_guard = Some(guard);
            plan.conditional_inputs.entry(expression).or_default().guards.push((guard, arms));
            conditional_input_certificates.insert(callee, certificate);
            forward_predecessors.entry(next).or_default().push((callee, slot));
            Some(endpoint)
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
        for (caller, consumer, ordinal, moved, origins) in users {
            if origins.is_empty() && !initializer_owned_formals.contains(&(callee, slot)) {
                continue;
            }
            let action = match origins.iter().next().filter(|_| origins.len() == 1) {
                Some(Some(InputOrigin::Plain(_))) => ClosedInput::Plain,
                Some(Some(InputOrigin::Formal(formal))) if matches!(moved, ArgumentProvenance::WholeMove(_))
                    && forward_edges.get(&(caller, *formal)) == Some(&(callee, slot))
                    && modes.get(&(caller, *formal)).is_some_and(|mode| mode.0)
                    && facts.raw_uses.is_empty()
                    && (ordinary_formal_outputs(&body.sites.boundary.root_outputs)
                        || (release.is_some()
                            && checked_scalar_output(&body.sites.boundary.root_outputs)
                            && facts.independent_checked_result
                            && bodies[&caller].facts.is_some_and(|facts| facts.checked_successor_result))) =>
                        ClosedInput::ForwardFormal(*formal),
                Some(Some(InputOrigin::ConditionalFormal(formal))) if matches!(moved, ArgumentProvenance::WholeMove(_))
                    && conditional_forward_edges.get(&(caller, *formal)) == Some(&(callee, slot))
                    && modes.get(&(caller, *formal)) == Some(&(true, false))
                    && !mixed && release.is_some() && facts.raw_uses.is_empty()
                    && ordinary_formal_outputs(&body.sites.boundary.root_outputs) =>
                        ClosedInput::ForwardFormalDeferred(*formal),
                Some(Some(InputOrigin::ConditionalFormal(formal))) =>
                    return Err(format!("aggregate owning formal conditional transfer has no exact release route in {:?} at slot {formal}",
                        bodies[&caller].source)),

                Some(source @ Some(InputOrigin::Owner(_))) if matches!(moved, ArgumentProvenance::WholeMove(_)) => {
                    let ArgumentProvenance::WholeMove(move_point) = moved else { unreachable!() };
                    let producer = active(caller, *source).ok_or_else(|| format!("aggregate owning input has unproved emitted incoming use in {:?} at call {ordinal} slot {slot}", bodies[&caller].source))?;
                    let call = bodies[&caller].facts.and_then(|f| f.calls.get(&producer)).ok_or_else(|| "missing owning input source receipt".to_owned())?;
                    let eligible = (release.is_some() && !mixed).then_some(TerminalInputSite {
                        ordinal, consumer, moved: move_point,
                    });
                    match discharged_by_inputs.entry((caller, producer)) {
                        std::collections::btree_map::Entry::Vacant(entry) => {
                            entry.insert([eligible, None]);
                        }
                        std::collections::btree_map::Entry::Occupied(mut entry)
                            if eligible.is_some() && entry.get()[0].is_some() && entry.get()[1].is_none() =>
                        {
                            entry.get_mut()[1] = eligible;
                        }
                        _ => return Err(format!("aggregate owning input has ambiguous source transfer in {:?} at call {ordinal}", bodies[&caller].source)),
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
                    if discharged_by_inputs.insert((caller, *producer), [None; 2]).is_some() {
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
        for &point in &facts.mixed_result_calls {
            if !facts.mixed_result_bindings.contains(&point)
                && target_body(&declarations, body.cfg, point).is_some_and(|target| outgoing.contains(&target))
            {
                return Err(format!("mixed aggregate result requires a matching local binding in {:?}", body.source));
            }
        }
        for (&call, &point) in &facts.unproved_receipt_loans {
            if recommitted.contains_key(&(key, call)) || target_body(&declarations, body.cfg, call).is_some_and(|target| outgoing.contains(&target)) {
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
            let route = if let Some(shape) = structured_returns.get(&key) {
                if !shape.complete || shape.leaves.is_empty() || shape.leaves.iter().any(|&leaf| {
                    target_body(&declarations, body.cfg, leaf).is_none_or(|target|
                        !outgoing.contains(&target) || bodies.get(&target).is_none_or(Body::has_mixed_struct_result))
                }) {
                    return Err(format!("structured aggregate return is not closed on every result path in {:?}", body.source));
                }
                ClosedReturn::Structured { output: shape.output.0 }
            } else {
                let mut values = body.return_owners();
                let Some(Some(owner)) = values.next() else {
                    return Err(format!(
                        "aggregate return owner is not established in {:?}",
                        body.source
                    ));
                };
                if values.any(|candidate| candidate != Some(owner)) {
                    if facts.mixed_branch_return.is_none() || body.return_owners().any(|owner| matches!(owner,
                        Some(OwnerSource::Call(point)) if recommitted.contains_key(&(key, point)))) {
                        return Err(format!("aggregate return owner has conflicting paths in {:?}", body.source));
                    }
                    ClosedReturn::MixedBranch
                } else {
                    match owner {
                        OwnerSource::Formal(_) => {
                            return Err(format!("return of moved formal is not closed in {:?}", body.source));
                        }
                        OwnerSource::Allocation(point) => {
                            let &(block, item) = facts.allocation_slots.get(&point).ok_or_else(|| {
                                format!("missing aggregate allocation binding in {:?}", body.source)
                            })?;
                            ClosedReturn::Allocation { block, item }
                        }
                        OwnerSource::Call(point) if recommitted.contains_key(&(key, point)) => {
                            let (call, output) = body.recommitted_root_output(point).ok_or_else(|| {
                                format!("recommitted aggregate return has no exact root binding in {:?}", body.source)
                            })?;
                            ClosedReturn::Recommit { call: call.0, output: output.0 }
                        }
                        OwnerSource::Call(point) => {
                            let target_mixed = target_body(&declarations, body.cfg, point)
                                .and_then(|target| bodies.get(&target)).is_some_and(Body::has_mixed_struct_result);
                            if (body.has_mixed_struct_result() || target_mixed)
                                && !(target_mixed && body.has_mixed_struct_result() && facts.mixed_forward == Some(point))
                            {
                                return Err(format!("forwarding a mixed aggregate result is not closed in {:?}", body.source));
                            }
                            let call = *facts.calls.get(&point).ok_or_else(|| {
                                format!(
                                    "missing aggregate forwarding occurrence in {:?}",
                                    body.source
                                )
                            })?;
                            ClosedReturn::Forward { call }
                        }
                    }
                }
            };
            Some(route)
        } else {
            None
        };
        let mut plan = result.bodies.remove(&key).unwrap_or_default();
        plan.returning = return_route;
        if return_route == Some(ClosedReturn::MixedBranch) {
            plan.mixed_branch_return = facts.mixed_branch_return.clone();
        }
        plan.input_calls = input_calls.remove(&key).unwrap_or_default();
        if let Some(&(slot, release, mixed)) = input_plans.get(&key) {
            plan.input = Some(slot);
            plan.input_mixed = mixed;
            if let Some((exit, block)) = release {
                plan.input_release = Some(exit);
                plan.scope_exits.insert(block, exit);
            }
        }
        // Index observed endpoints once. Looking up every owner by scanning all
        // scopes would make a chain of otherwise linear certificates quadratic.
        let mut call_endpoints = BTreeMap::<_, Vec<_>>::new();
        if !facts.calls.is_empty() {
            for (&endpoint, receipts) in &facts.endpoints {
                for (&owner, witness) in &receipts.possible {
                    if let OwnerSource::Call(point) = owner {
                        call_endpoints.entry(point).or_default()
                            .push((endpoint, receipts.always_retains(owner, witness.as_ref())));
                    }
                }
            }
        }
        let mut conditional_certificate = conditional_input_certificates.remove(&key);
        let mut shared_conditional_guards = plan.conditional_inputs.iter().flat_map(|(&expression, inputs)| {
            inputs.guards.iter().map(move |&(guard, arms)| ((expression, arms), guard))
        }).collect::<BTreeMap<_, _>>();
        let mut shared_transfer_coverage = BTreeMap::new();
        let orders_by_birth = recommitted.range((key, CfgPointId(0))..=(key, CfgPointId(usize::MAX)))
            .next().is_some();
        let mut ranked_releases = BTreeMap::<CfgPointId, BTreeMap<CfgPointId, ClosedOwnerRelease>>::new();
        let mut record_release = |plan: &mut ClosedBody, exit, birth, release| {
            if orders_by_birth {
                if ranked_releases.entry(exit).or_default().insert(birth, release).is_some() {
                    return Err(format!("aggregate release has conflicting original receipts in {:?}", body.source));
                }
            } else {
                plan.releases.entry(exit).or_default().push(release);
            }
            Ok(())
        };
        for (&point, &call) in &facts.calls {
            let previous = recommitted.get(&(key, point));
            if let Some(origin) = previous {
                plan.recommits.insert(call.0, facts.calls[&origin.previous].0);
            } else if !target_body(&declarations, body.cfg, point)
                .is_some_and(|target| outgoing.contains(&target))
            {
                continue;
            }
            if matches!(return_route, Some(ClosedReturn::Forward { call: forwarded }) if call == forwarded)
            {
                plan.calls.insert(call.0, ClosedCall::Forward);
                continue;
            }
            let owner = OwnerSource::Call(point);
            let mut endpoints = call_endpoints.get(&point).into_iter().flatten().copied();
            if matches!(return_route, Some(ClosedReturn::Structured { .. }))
                && structured_returns[&key].leaves.contains(&point)
            {
                if previous.is_some() || discharged_by_inputs.contains_key(&(key, point)) || endpoints.next().is_some() {
                    return Err(format!("structured aggregate result has competing owner routes in {:?}", body.source));
                }
                if let Some(&(block, output)) = structured_returns[&key].named.get(&point) {
                    plan.named_result_arms.insert(block.0, (output.0, call.0));
                    plan.calls.insert(call.0, ClosedCall::Receive);
                } else {
                    plan.calls.insert(call.0, ClosedCall::ResultArm);
                }
                continue;
            }

            if matches!(return_route, Some(ClosedReturn::Recommit { call: returned, .. }) if returned == call.0) {
                if previous.is_none() || discharged_by_inputs.contains_key(&(key, point)) || endpoints.next().is_some() {
                    return Err(format!("recommitted aggregate return has competing ownership routes in {:?}", body.source));
                }
                continue;
            }

            if let Some(&[first, second]) = discharged_by_inputs.get(&(key, point)) {
                let endpoint = endpoints.next();
                if endpoint.is_some() || second.is_some() {
                    let ordinary_producer = target_body(&declarations, body.cfg, point)
                        .and_then(|target| bodies.get(&target))
                        .is_some_and(|producer| !producer.has_mixed_struct_result());
                    let points = &body.cfg.points;
                    let certificate = conditional_certificate.get_or_insert_with(|| {
                        ConditionalInputCertificate::build(points, &facts.conditional_input_regions, &facts.conditional_input_barriers)
                    });
                    let uncovered = || {
                        let mut message = format!(
                            "aggregate input transfer does not cover every owning path in {:?}", body.source
                        );
                        if certificate.depth_limit_reached {
                            message.push_str(&format!(
                                "; structured receipt certification reached its nesting limit of {MAX_RECEIPT_FORK_DEPTH}"
                            ));
                        }
                        message
                    };
                    if returning || !ordinary_producer || endpoints.next().is_some() {
                        return Err(uncovered());
                    }
                    let primary = first.ok_or_else(uncovered)?;
                    let exit_point = endpoint.map(|((exit, _), _)| exit);
                    let install = if points[point.0].scope == points[0].scope {
                        None // Retain the existing root receipt routes.
                    } else {
                        Some(facts.direct_receipt_bindings.get(&point).ok_or_else(uncovered)?)
                    };
                    let classify = |candidate: TerminalInputSite| {
                        let site = certificate.consuming_arm(points, point, candidate.consumer, exit_point)?;
                        if let Some((install, binding)) = install {
                            let branch = certificate.forks[site.2].region.branch;
                            let moved = candidate.moved;
                            if *install <= point || *install >= branch || *install >= moved
                                || points[install.0].scope != points[point.0].scope
                                || certificate.paths[install.0] != certificate.paths[point.0]
                                || !matches!(&points[moved.0].kind, CfgPointKind::Move { owner, .. } if owner == binding)
                            {
                                return None;
                            }
                        }
                        Some(site)
                    };
                    let first = classify(primary).ok_or_else(uncovered)?;
                    let second_arm = match second {
                        Some(candidate) => Some(classify(candidate).ok_or_else(uncovered)?),
                        None => None,
                    };
                    let sites = match second_arm {
                        Some(other) if other < first => (other, Some(first)),
                        _ => (first, second_arm),
                    };
                    let (expression, gate) = if let Some(&coverage) = shared_transfer_coverage.get(&sites) {
                        coverage
                    } else {
                        let merges = certificate.merge_transfers(sites.0, sites.1).ok_or_else(uncovered)?;
                        let mut merge_guards = BTreeMap::new();
                        let mut final_gate = None;
                        for (fork, arms) in merges {
                            let expression = certificate.forks[fork].region.expression.0;
                            let arms = arms.map(|gate| match gate {
                                ReceiptGate::Guard(child) => ReceiptGate::Guard(merge_guards[&child]),
                                constant => constant,
                            });
                            let gate = if arms[0] == arms[1] {
                                arms[0]
                            } else {
                                let guard = *shared_conditional_guards.entry((expression, arms)).or_insert_with(|| {
                                    let guard = plan.conditional_guard_count;
                                    plan.conditional_guard_count += 1;
                                    plan.conditional_inputs.entry(expression).or_default().guards.push((guard, arms));
                                    guard
                                });
                                ReceiptGate::Guard(guard)
                            };
                            if let ReceiptGate::Guard(guard) = gate {
                                merge_guards.insert(fork, guard);
                            }
                            if fork == sites.0.2 {
                                final_gate = Some((expression, gate));
                            }
                        }
                        let coverage = final_gate.ok_or_else(uncovered)?;
                        shared_transfer_coverage.insert(sites, coverage);
                        coverage
                    };
                    match (endpoint, gate) {
                        (Some((endpoint @ (exit, block), false)), ReceiptGate::Guard(guard))
                            if facts.endpoints.get(&endpoint)
                                .is_some_and(|endpoint| endpoint.has_conditional_unique_receipt(owner)
                                    && install.is_none_or(|(_, binding)|
                                        endpoint.possible.get(&owner).and_then(Option::as_ref) == Some(binding))) =>
                        {
                            if plan.scope_exits.insert(block, exit).is_some_and(|old| old != exit) {
                                return Err(format!("aggregate release has conflicting scope exits in {:?}", body.source));
                            }
                            let birth = previous.map_or(point, |origin| origin.birth);
                            record_release(&mut plan, exit, birth, ClosedOwnerRelease { receipt: call.0, guard: Some(guard) })?;
                        }
                        (None, ReceiptGate::Transferred) => {
                            plan.conditional_inputs.entry(expression).or_default().consumed_on_both_arms.push(call.0);
                        }
                        _ => return Err(uncovered()),
                    }
                    if let Some(other) = second {
                        plan.deferred_call_inputs.insert(other.ordinal, call.0);
                    }
                    plan.deferred_call_inputs.insert(primary.ordinal, call.0);
                }
                if previous.is_none() { plan.calls.insert(call.0, ClosedCall::Receive); }
                continue;
            }
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
            if previous.is_none() { plan.calls.insert(call.0, ClosedCall::Receive); }
            if plan.scope_exits.insert(block, exit).is_some_and(|old| old != exit) {
                return Err(format!("aggregate release has conflicting scope exits in {:?}", body.source));
            }
            let birth = previous.map_or(point, |origin| origin.birth);
            record_release(&mut plan, exit, birth, ClosedOwnerRelease { receipt: call.0, guard: None })?;
        }
        plan.releases.extend(ranked_releases.into_iter().map(|(exit, releases)|
            (exit, releases.into_values().collect())));
        if (returning || !plan.calls.is_empty() || !plan.recommits.is_empty() || plan.input.is_some() || !plan.input_calls.is_empty()) && !body.is_monomorphic {
            return Err(format!("aggregate ownership in generic reservation {key:?} is not closed in {:?}", body.source));
        }
        if (returning || !plan.calls.is_empty() || !plan.recommits.is_empty() || plan.input.is_some() || !plan.input_calls.is_empty()) && !facts.unsupported.is_empty() {
            return Err(format!(
                "aggregate transfer has unsupported use in {:?} at point {:?}",
                body.source,
                facts.unsupported.first()
            ));
        }
        if returning || !plan.calls.is_empty() || !plan.recommits.is_empty() || plan.input.is_some() || !plan.input_calls.is_empty()
            || plan.identity_relay || !plan.same_array_rebinds.is_empty() || !plan.fresh_array_recommits.is_empty() || plan.scalar_descriptor_assignment.is_some()
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
    finalize_cohorts(result)
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
    fn indirect_fresh_array_use_requires_the_same_normalized_signature() {
        let row = |element: ScalarType, count: usize, result: ScalarType, span: ByteSpan| CheckedCallSignature {
            parameters: std::iter::once(ScalarType::RuntimeArray { element: Box::new(element.clone()), span })
                .chain(std::iter::repeat_n(ScalarType::U64, count)).collect(),
            outputs: ScalarOutputSequence {
                outputs: vec![ScalarOutput { ty: ScalarType::RuntimeArray { element: Box::new(element), span }, span },
                    ScalarOutput { ty: result, span }],
                span,
            },
            overload: None,
            overload_origin: None,
        };
        let base = row(ScalarType::U8, 3, ScalarType::U64, ByteSpan::new(1, 4));
        let candidates = HashSet::from([fresh_recursive_call_signature(
            base.parameters.iter(), base.outputs.outputs.iter().map(|output| &output.ty),
        )]);
        let other_site = ByteSpan::new(5, 9);
        assert!(could_call_fresh_recursive_array(None, &candidates));
        assert!(could_call_fresh_recursive_array(Some(&row(ScalarType::U8, 3, ScalarType::U64, other_site)), &candidates));
        assert!(!could_call_fresh_recursive_array(Some(&row(ScalarType::U16, 3, ScalarType::U64, other_site)), &candidates));
        assert!(!could_call_fresh_recursive_array(Some(&row(ScalarType::U8, 2, ScalarType::U64, other_site)), &candidates));
        assert!(!could_call_fresh_recursive_array(Some(&row(ScalarType::U8, 3, ScalarType::Bool, other_site)), &candidates));
    }

    #[test]
    fn recursive_array_replacement_closes_typed_owner_receipts() {
        let text = include_str!("../../../tests/fixtures/recursive-replacement.w");
        let source = SourceIdentity::new("project".into(), "workbench".into(), "src/recursive_replacement.w".into(), "r1".into());
        let parsed = crate::parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let prepared = prepare_scalar_project(
            vec![(derive_scalar_declarations_from_cst(parsed.result.canonical_cst()), Vec::new())],
            vec![source.clone()],
            ScalarPreparationInputs { target_layout: ScalarTargetLayout::NATIVE64 },
        ).unwrap_or_else(|failure| panic!("{:?}", failure.diagnostics));
        let module = &prepared.project().modules[0];
        let mut results = BTreeMap::new();
        let mut declarations = HashMap::new();
        let mut lookup = HashMap::new();
        for (item, declaration) in module.items.iter().enumerate() {
            let ScalarItem::Function(function) = declaration else { continue };
            let key = (0, item, None);
            let sites = prepared.function_calls(0, item);
            let mut errors = Vec::new();
            let result = derive_function_reference_result(&source, function, &HashSet::new(), sites,
                &module.structs, &module.enums, None, true, &mut errors);
            assert!(errors.is_empty(), "{errors:?}");
            results.insert(key, (result, sites));
            lookup.insert(function.name.as_str(), key);
            declarations.insert((&source, function.name_span), (0, item));
        }
        let mut bodies = BTreeMap::new();
        for (&key, (result, sites)) in &results {
            bodies.insert(key, Body { facts: Some(&result.aggregate), cfg: &result.cfg, sites, source: &source,
                is_monomorphic: true, supports_input_result: false });
        }
        let leaf = &results[&lookup["replace_owner"]].0;
        assert_eq!(leaf.aggregate.fresh_recommit_leaf, Some(RecommittedArrayExtent::Formal(1)),
            "fresh leaf declined: {:?}", leaf.cfg.initial_points);
        let key = lookup["recursive_return"];
        let recursive = &results[&key].0;
        let facts = &recursive.aggregate;
        assert!(facts.array_neutral && !facts.array_invalid && !facts.array_conditional_forwarded,
            "recursive effects declined: neutral={}, invalid={}, conditional={}; points={:?}",
            facts.array_neutral, facts.array_invalid, facts.array_conditional_forwarded, recursive.cfg.initial_points);
        assert_eq!(facts.array_returns.len(), 2, "return paths: {:?}", facts.array_returns);
        assert_eq!(facts.array_rebinds.len(), 2, "recommits: {:?}", facts.array_rebinds);
        assert_eq!(facts.array_calls.len(), 2, "calls: {:?}", facts.array_calls);
        assert_eq!(fresh_recursive_recommit(&bodies[&key], key, &bodies, &declarations),
            Some(RecommittedArrayExtent::Formal(2)), "recursive result declined: {:?}", facts.array_returns);
        let caller = &results[&lookup["recursive_replacement"]].0;
        assert_eq!(caller.aggregate.array_rebinds.len(), 1, "outer recommit declined: {:?}", caller.cfg.initial_points);
        assert_eq!(caller.aggregate.allocation_slots.len(), 1, "outer allocation: {:?}", caller.aggregate.allocation_slots);

        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        let reservations = AggregateReservations {
            bodies: lookup.values().copied().map(|key| (key, true)).collect(),
            modules: vec![0],
        };
        let plans = build.aggregate(&reservations).expect("actual build ownership plans");
        let forwards = &plans.bodies[&key].fresh_array_recommits;
        assert_eq!(forwards.len(), 2);
        assert!(forwards.values().all(|receipt| receipt.forward));
        let receivers = &plans.bodies[&lookup["recursive_replacement"]].fresh_array_recommits;
        assert_eq!(receivers.len(), 1);
        assert!(receivers.values().all(|receipt| !receipt.forward));
    }

    fn synthetic_literal_array_pair() -> LiteralArrayPair {
        let allocation = |bind, length, elements| LiteralArrayBind {
            bind: CfgPointId(bind),
            placeholder: CfgPointId(bind - 1),
            position: BlockPosition { block: BlockOrdinal(0), item: 0 },
            length_occurrence: OccurrenceOrdinal(length),
            extent: FreshArrayExtent::Literal(elements),
        };
        LiteralArrayPair {
            caller: (0, 0, None),
            callee: (0, 1, None),
            established_alternate: None,
            original: allocation(2, 1, 3),
            fresh: allocation(3, 2, 11),
            final_output: OccurrenceOrdinal(3),
            write: LiteralArrayWrite {
                rhs: CfgPointId(3),
                writer: CfgPointId(4),
                producer: OccurrenceOrdinal(2),
                destination: OccurrenceOrdinal(3),
                argument: None,
            },
            exit: CfgPointId(5),
            block: BlockOrdinal(0),
            unit: None,
        }
    }

    #[test]
    fn literal_array_finalizer_selects_only_proved_monomorphic_callers() {
        let pair = synthetic_literal_array_pair();
        let mut facts = AggregateFlowResult { returning_leaf: true, ..AggregateFlowResult::default() };
        assert!(facts.has_observations());
        assert!(facts.proved_literal_array_caller(pair.caller, true).is_none());

        facts.literal_array_route = Some(LiteralArrayRoute::FreshLeaf {
            allocation: pair.fresh, output: pair.final_output,
        });
        assert!(facts.proved_literal_array_caller(pair.callee, true).is_none());

        facts.literal_array_route = Some(LiteralArrayRoute::Replacement {
            original: pair.original, write: pair.write, exit: pair.exit, block: pair.block, unit: None,
        });
        let route = facts.proved_literal_array_caller(pair.caller, true).expect("proved caller");
        assert!(std::ptr::eq(route, facts.literal_array_route.as_ref().unwrap()));
        assert!(facts.proved_literal_array_caller(pair.caller, false).is_none());
        assert!(facts.proved_literal_array_caller((pair.caller.0, pair.caller.1, Some(0)), true).is_none());
        facts.literal_array_unknown = true;
        assert!(facts.proved_literal_array_caller(pair.caller, true).is_none());
    }

    #[test]
    fn aggregate_observation_ignores_incomplete_literal_candidates() {
        let pair = synthetic_literal_array_pair();
        let span = ByteSpan::new(0, 0);
        let binding = ReferenceBindingId {
            source: SourceIdentity::new("project".into(), "package".into(), "metadata".into(), "r1".into()),
            function_span: span, declaration_span: span, block_span: span, kind: ReferenceBindingKind::Declared,
        };
        let mut facts = AggregateFlowFacts {
            allocations: BTreeMap::from([(binding, pair.original.bind)]),
            literal_array_bind: Some(pair.original),
            ..AggregateFlowFacts::default()
        };
        assert_eq!(facts.observation_mode(), None);
        facts.calls.insert(pair.write.rhs, pair.write.producer);
        assert_eq!(facts.observation_mode(), Some(AggregateObservationMode { literal: false, legacy: true }));
        facts.literal_array_write = Some(pair.write);
        facts.literal_array_writer = Some(BlockOrdinal(1));
        assert_eq!(facts.observation_mode(), Some(AggregateObservationMode { literal: false, legacy: true }));
        facts.literal_array_writer = None;
        assert_eq!(facts.observation_mode(), Some(AggregateObservationMode { literal: true, legacy: true }));
        facts.calls.clear();
        assert_eq!(facts.observation_mode(), Some(AggregateObservationMode { literal: true, legacy: false }));
        facts.literal_array_write = None;
        facts.literal_array_value = Some((pair.write.rhs, pair.final_output));
        assert_eq!(facts.observation_mode(), Some(AggregateObservationMode { literal: true, legacy: false }));
    }

    #[test]
    fn checked_literal_array_pair_enrolls_distinct_receipts_at_the_exact_exit() {
        let pair = synthetic_literal_array_pair();
        let mut plans = ClosedAggregates::default();
        enroll_literal_array_pair(&mut plans, pair, LiteralArrayParameters::default());
        assert_eq!(plans.bodies.len(), 2);
        assert_eq!(plans.bodies[&pair.callee].literal_array, Some(ClosedLiteralArray::FreshLeaf {
            receipt: OriginalArrayReceipt { origin: CfgPointId(3), allocation: (0, 0) },
            length_occurrence: 2,
            extent: FreshArrayExtent::Literal(11),
            output: 3,
        }));
        let caller = &plans.bodies[&pair.caller];
        assert_eq!(caller.literal_array, Some(ClosedLiteralArray::Replacement {
            original: OriginalArrayReceipt { origin: CfgPointId(2), allocation: (0, 0) },
            original_length_occurrence: 1,
            original_elements: 3,
            call: 2,
            destination: 3,
            exit: CfgPointId(5),
            block: 0,
            extent: ReplacementArrayExtent::Literal(11),
            unit: None,
        }));
        assert_eq!(caller.scope_exits, BTreeMap::from([(0, CfgPointId(5))]));
        assert!(caller.releases.is_empty());

        let mut self_pair = ClosedAggregates::default();
        enroll_literal_array_pair(&mut self_pair, LiteralArrayPair { callee: pair.caller, ..pair },
            LiteralArrayParameters::default());
        assert!(self_pair.bodies.is_empty());

        for occupied in [pair.caller, pair.callee] {
            let mut overlapping = ClosedAggregates::default();
            overlapping.bodies.entry(occupied).or_default().ordinary_result = true;
            enroll_literal_array_pair(&mut overlapping, pair, LiteralArrayParameters::default());
            assert_eq!(overlapping.bodies.len(), 1);
            assert!(overlapping.bodies[&occupied].ordinary_result);
            assert!(overlapping.bodies[&occupied].literal_array.is_none());
        }
    }

    #[test]
    fn literal_array_preflight_is_atomic_and_reuses_only_the_exact_leaf() {
        let pair = synthetic_literal_array_pair();
        let parameters = LiteralArrayParameters::default();
        let mut result = ClosedAggregates::default();
        let prepared = preflight_literal_array_pair(&result, pair, parameters).expect("valid pair");
        assert!(matches!(prepared.new_leaf, Some((key, ClosedLiteralArray::FreshLeaf {
            extent: FreshArrayExtent::Literal(11), ..
        })) if key == pair.callee));
        assert!(result.bodies.is_empty());
        enroll_literal_array_pair(&mut result, pair, parameters);

        let next = LiteralArrayPair { caller: (0, 2, None), ..pair };
        let leaf = result.bodies[&pair.callee].literal_array;
        let mut incompatible = next;
        incompatible.fresh.extent = FreshArrayExtent::Literal(12);
        assert!(preflight_literal_array_pair(&result, incompatible, parameters).is_none());
        enroll_literal_array_pair(&mut result, incompatible, parameters);
        assert_eq!(result.bodies.len(), 2);
        assert_eq!(result.bodies[&pair.callee].literal_array, leaf);
        assert_eq!(result.bodies[&pair.caller].scope_exits, BTreeMap::from([(pair.block.0, pair.exit)]));

        let prepared = preflight_literal_array_pair(&result, next, parameters).expect("same leaf");
        assert!(prepared.new_leaf.is_none());
        assert_eq!(result.bodies.len(), 2);
        enroll_literal_array_pair(&mut result, next, parameters);
        assert_eq!(result.bodies.len(), 3);
        assert_eq!(result.bodies[&next.caller].scope_exits, BTreeMap::from([(next.block.0, next.exit)]));
        assert_eq!(result.bodies[&pair.callee].literal_array, leaf);
    }

    #[test]
    fn distinct_literal_array_preflight_reuses_both_established_leaves() {
        let mut pair = synthetic_literal_array_pair();
        let parameters = LiteralArrayParameters::default();
        let alternate = (0, 2, None);
        let mut alternate_allocation = pair.fresh;
        alternate_allocation.bind = CfgPointId(4);
        alternate_allocation.extent = FreshArrayExtent::Literal(13);
        let alternate_output = OccurrenceOrdinal(12);
        pair.established_alternate = Some((alternate, alternate_allocation, alternate_output));
        pair.unit = Some(LiteralArrayUnit {
            expression: OccurrenceOrdinal(4), condition: CfgPointId(4), operands: None, branch: CfgPointId(5),
            writer_side: ArrayWriterSide::Then, writer_entry: CfgPointId(6), writer_exit: CfgPointId(9),
            writer_block: BlockOrdinal(1), writer_scope: ReferenceScopeId(1),
            retained: Some(LiteralArrayRetained {
                entry: CfgPointId(10), exit: CfgPointId(13), block: BlockOrdinal(2), scope: ReferenceScopeId(2),
            }),
            other_write: Some(LiteralArrayWrite {
                rhs: CfgPointId(11), writer: CfgPointId(12), producer: OccurrenceOrdinal(7),
                destination: OccurrenceOrdinal(8), argument: None,
            }),
            join: CfgPointId(14),
        });
        pair.write.rhs = CfgPointId(7);
        pair.write.writer = CfgPointId(8);
        pair.exit = CfgPointId(15);

        let mut result = ClosedAggregates::default();
        enroll_literal_array_pair(&mut result, pair, parameters);
        assert!(result.bodies.is_empty());
        let primary_seed = LiteralArrayPair { caller: (0, 3, None), established_alternate: None, unit: None, ..pair };
        enroll_literal_array_pair(&mut result, primary_seed, parameters);
        enroll_literal_array_pair(&mut result, pair, parameters);
        assert_eq!(result.bodies.len(), 2);
        assert!(!result.bodies.contains_key(&pair.caller) && !result.bodies.contains_key(&alternate));

        let mut alternate_seed = LiteralArrayPair {
            caller: (0, 4, None), callee: alternate, fresh: alternate_allocation, final_output: alternate_output,
            ..primary_seed
        };
        alternate_seed.fresh.extent = FreshArrayExtent::Literal(12);
        let incompatible = preflight_literal_array_pair(&result, alternate_seed, parameters).unwrap().new_leaf.unwrap().1;
        result.bodies.entry(alternate).or_default().literal_array = Some(incompatible);
        enroll_literal_array_pair(&mut result, pair, parameters);
        assert_eq!(result.bodies.len(), 3);
        assert!(!result.bodies.contains_key(&pair.caller));
        assert_eq!(result.bodies[&alternate].literal_array, Some(incompatible));

        result.bodies.remove(&alternate);
        alternate_seed.fresh = alternate_allocation;
        enroll_literal_array_pair(&mut result, alternate_seed, parameters);
        let primary_plan = result.bodies[&pair.callee].literal_array;
        let alternate_plan = result.bodies[&alternate].literal_array;
        let prepared = preflight_literal_array_pair(&result, pair, parameters).expect("both exact leaves");
        assert!(prepared.new_leaf.is_none());
        assert!(matches!(prepared.replacement, ClosedLiteralArray::Replacement {
            extent: ReplacementArrayExtent::Literal(11),
            unit: Some(ClosedLiteralArrayUnit {
                other_writer: Some(ClosedLiteralArrayWrite {
                    call: 7, destination: 8, extent: ReplacementArrayExtent::Literal(13),
                }),
                ..
            }),
            ..
        }));
        enroll_literal_array_pair(&mut result, pair, parameters);
        assert_eq!(result.bodies.len(), 5);
        assert_eq!(result.bodies[&pair.callee].literal_array, primary_plan);
        assert_eq!(result.bodies[&alternate].literal_array, alternate_plan);
        let caller = &result.bodies[&pair.caller];
        assert_eq!(caller.scope_exits, BTreeMap::from([(pair.block.0, pair.exit)]));
        assert!(caller.releases.is_empty() && caller.input_release.is_none());
    }

    #[test]
    fn checked_formal_extent_enrolls_only_the_exact_u64_argument() {
        let mut pair = synthetic_literal_array_pair();
        pair.fresh.extent = FreshArrayExtent::Formal;
        pair.write.argument = Some(LiteralArrayArgument { read: CfgPointId(3), occurrence: OccurrenceOrdinal(9) });
        pair.write.rhs = CfgPointId(4);
        pair.write.writer = CfgPointId(5);
        pair.write.destination = OccurrenceOrdinal(10);
        pair.exit = CfgPointId(6);
        let formal = |ty| ParameterSlot {
            ordinal: 0,
            name: String::new(),
            span: ByteSpan::new(0, 0),
            ty,
        };
        let u64_slot = [formal(ScalarType::U64)];
        let u32_slot = [formal(ScalarType::U32)];
        let exact = LiteralArrayParameters {
            caller: &u64_slot,
            callee: &u64_slot,
            signature: &[ScalarType::U64],
        };

        let mut plans = ClosedAggregates::default();
        enroll_literal_array_pair(&mut plans, pair, exact);
        assert_eq!(plans.bodies.len(), 2);
        assert!(matches!(plans.bodies[&pair.callee].literal_array,
            Some(ClosedLiteralArray::FreshLeaf {
                receipt: OriginalArrayReceipt { origin: CfgPointId(3), .. },
                extent: FreshArrayExtent::Formal, ..
            })));
        let caller = &plans.bodies[&pair.caller];
        assert!(matches!(caller.literal_array, Some(ClosedLiteralArray::Replacement {
            original: OriginalArrayReceipt { origin: CfgPointId(2), .. },
            original_elements: 3, destination: 10,
            extent: ReplacementArrayExtent::Argument { occurrence: 9 }, ..
        })));
        assert_eq!(caller.scope_exits, BTreeMap::from([(pair.block.0, pair.exit)]));
        assert!(caller.releases.is_empty() && caller.input_release.is_none());

        let mut conflicting = pair;
        conflicting.caller = (0, 2, None);
        conflicting.fresh.extent = FreshArrayExtent::Literal(11);
        enroll_literal_array_pair(&mut plans, conflicting, exact);
        assert_eq!(plans.bodies.len(), 2);
        assert!(!plans.bodies.contains_key(&conflicting.caller));
        assert!(matches!(plans.bodies[&pair.callee].literal_array,
            Some(ClosedLiteralArray::FreshLeaf { extent: FreshArrayExtent::Formal, .. })));

        for rejected in [
            LiteralArrayParameters { callee: &u32_slot, ..exact },
            LiteralArrayParameters { caller: &u32_slot, callee: &u32_slot, signature: &[ScalarType::U32] },
        ] {
            let mut plans = ClosedAggregates::default();
            enroll_literal_array_pair(&mut plans, pair, rejected);
            assert!(plans.bodies.is_empty());
        }

        let mut dual = pair;
        dual.write.rhs = CfgPointId(8);
        dual.write.writer = CfgPointId(9);
        dual.write.argument = Some(LiteralArrayArgument { read: CfgPointId(7), occurrence: OccurrenceOrdinal(9) });
        dual.exit = CfgPointId(18);
        let other = LiteralArrayWrite {
            rhs: CfgPointId(13), writer: CfgPointId(14),
            producer: OccurrenceOrdinal(11), destination: OccurrenceOrdinal(13),
            argument: Some(LiteralArrayArgument { read: CfgPointId(12), occurrence: OccurrenceOrdinal(12) }),
        };
        let unit = LiteralArrayUnit {
            expression: OccurrenceOrdinal(4), condition: CfgPointId(4), operands: None, branch: CfgPointId(5),
            writer_side: ArrayWriterSide::Then, writer_entry: CfgPointId(6), writer_exit: CfgPointId(10),
            writer_block: BlockOrdinal(1), writer_scope: ReferenceScopeId(1),
            retained: Some(LiteralArrayRetained {
                entry: CfgPointId(11), exit: CfgPointId(15), block: BlockOrdinal(2), scope: ReferenceScopeId(2),
            }),
            other_write: Some(other),
            join: CfgPointId(17),
        };
        dual.unit = Some(unit);
        let mut plans = ClosedAggregates::default();
        enroll_literal_array_pair(&mut plans, dual, exact);
        let caller = &plans.bodies[&dual.caller];
        assert!(matches!(caller.literal_array, Some(ClosedLiteralArray::Replacement {
            extent: ReplacementArrayExtent::Argument { occurrence: 9 },
            unit: Some(ClosedLiteralArrayUnit {
                other_writer: Some(ClosedLiteralArrayWrite {
                    call: 11, destination: 13, extent: ReplacementArrayExtent::Argument { occurrence: 12 },
                }), ..
            }), ..
        })));
        assert_eq!(caller.scope_exits, BTreeMap::from([(dual.block.0, dual.exit)]));
        assert!(caller.releases.is_empty() && caller.input_release.is_none());

        for argument in [None, dual.write.argument] {
            let mut plans = ClosedAggregates::default();
            let unit = LiteralArrayUnit {
                other_write: Some(LiteralArrayWrite { argument, ..other }), ..unit
            };
            enroll_literal_array_pair(&mut plans, LiteralArrayPair { unit: Some(unit), ..dual }, exact);
            assert!(plans.bodies.is_empty());
        }
    }

    #[test]
    fn prepared_array_unit_cfg_enrolls_one_owner_exit_for_both_paths() {
        let source = SourceIdentity::new("project".into(), "package".into(), "metadata".into(), "r1".into());
        let span = ByteSpan::new(0, 0);
        let binding = ReferenceBindingId {
            source: source.clone(), function_span: span, declaration_span: span,
            block_span: span, kind: ReferenceBindingKind::Declared,
        };
        let place = ReferencePlaceId {
            source: source.clone(), function_span: span, declaration_span: span, block_span: span,
            binding: binding.clone(), projections: Vec::new(),
            place: ScalarPlace::Name { name: String::new(), span },
        };
        let unit = LiteralArrayUnit {
            expression: OccurrenceOrdinal(4), condition: CfgPointId(4), operands: None, branch: CfgPointId(5),
            writer_side: ArrayWriterSide::Then, writer_entry: CfgPointId(6), writer_exit: CfgPointId(9), writer_block: BlockOrdinal(1),
            writer_scope: ReferenceScopeId(1), retained: None, other_write: None, join: CfgPointId(10),
        };
        let mut pair = synthetic_literal_array_pair();
        pair.original.placeholder = CfgPointId(2);
        pair.original.bind = CfgPointId(3);
        pair.write.rhs = CfgPointId(7);
        pair.write.writer = CfgPointId(8);
        pair.write.producer = OccurrenceOrdinal(5);
        pair.write.destination = OccurrenceOrdinal(6);
        pair.exit = CfgPointId(11);
        let mut facts = AggregateFlowFacts {
            allocations: BTreeMap::from([(binding.clone(), pair.original.bind)]),
            literal_array_bind: Some(pair.original),
            literal_array_write: Some(pair.write),
            literal_array_unit: Some(unit),
            scope_exits: BTreeMap::from([(pair.exit, pair.block)]),
            ..AggregateFlowFacts::default()
        };
        let boolean = || ScalarExpression::Boolean { value: false, span };
        let point = |id, scope, kind, successors: &[usize]| ReferenceFlowPoint {
            id: CfgPointId(id), kind, source_span: SourceSpan::new(source.clone(), span),
            scope: ReferenceScopeId(scope), possible_origins: None,
            successors: successors.iter().copied().map(CfgPointId).collect(),
        };
        // These are compiler nodes only; no parser or emitter participates.
        let points = [
            point(0, 0, CfgPointKind::Entry, &[1]),
            point(1, 0, CfgPointKind::ScopeEnter, &[2]),
            point(2, 0, CfgPointKind::Evaluate(boolean()), &[3]),
            point(3, 0, CfgPointKind::Bind { binding: binding.clone() }, &[4]),
            point(4, 0, CfgPointKind::Evaluate(boolean()), &[5]),
            point(5, 0, CfgPointKind::Branch { condition: boolean() }, &[6, 10]),
            point(6, 1, CfgPointKind::ScopeEnter, &[7]),
            point(7, 1, CfgPointKind::Call {
                expression: ScalarExpression::Call {
                    receiver: None, name: String::new(), receiver_span: None, name_span: span,
                    type_arguments: Vec::new(), arguments: Vec::new(), overload_selection: None, span,
                },
                output: 0, argument_points: Vec::new(),
                target: ResolvedCallTarget::ModuleCallable {
                    source: source.clone(), declaration_span: span, concrete: ConcreteCallSelection::Declaration,
                },
            }, &[8]),
            point(8, 1, CfgPointKind::Assign {
                target: place, rhs_point: pair.write.rhs,
                previous_origins: BTreeSet::new(), previous_loans: Vec::new(),
            }, &[9]),
            point(9, 1, CfgPointKind::ScopeExit, &[10]),
            point(10, 0, CfgPointKind::Join, &[11]),
            point(11, 0, CfgPointKind::ScopeExit, &[]),
        ];
        let observe = |facts: &AggregateFlowFacts, points: &[ReferenceFlowPoint], path: &[usize],
                       result: &mut AggregateFlowResult| {
            let mut state = AggregateEdgeState::default();
            for &id in path {
                facts.visit_literal_array(&points[id], &mut state, result);
            }
        };
        let writer = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
        let fallthrough = [0, 1, 2, 3, 4, 5, 10, 11];
        let mut observed = AggregateFlowResult::default();
        observe(&facts, &points, &writer, &mut observed);
        assert!(observed.proved_literal_array_route().is_none());
        observe(&facts, &points, &fallthrough, &mut observed);
        let Some(LiteralArrayRoute::Replacement { original, write, exit, block, unit: proved }) =
            observed.proved_literal_array_route() else { panic!("missing checked joined ownership") };
        assert_eq!((original, write, exit, block, proved),
            (pair.original, pair.write, pair.exit, pair.block, Some(unit)));

        let mut plans = ClosedAggregates::default();
        enroll_literal_array_pair(&mut plans, LiteralArrayPair { original, write, exit, block, unit: proved, ..pair },
            LiteralArrayParameters::default());
        let caller = &plans.bodies[&pair.caller];
        assert!(matches!(caller.literal_array, Some(ClosedLiteralArray::Replacement {
            unit: Some(ClosedLiteralArrayUnit { expression: 4, writer_block: 1, writer_side: ArrayWriterSide::Then, retained_block: None, other_writer: None }), ..
        })));
        assert_eq!(caller.scope_exits, BTreeMap::from([(0, CfgPointId(11))]));
        assert!(caller.releases.is_empty() && caller.input_release.is_none());

        // Exercise the same visitor with the two exact operand points.
        let mut formal = binding.clone();
        formal.declaration_span = ByteSpan::new(1, 2);
        let integer = ScalarExpression::Integer { value: 0.into(), span };
        let comparison = ScalarExpression::Binary {
            operator: BinaryOperator::Equal,
            left: Box::new(ScalarExpression::Name { name: String::new(), span }),
            right: Box::new(integer.clone()),
            span,
        };
        let mut compared = points.to_vec();
        compared[3].successors = vec![CfgPointId(12)];
        compared[4].kind = CfgPointKind::Evaluate(comparison.clone());
        compared[5].kind = CfgPointKind::Branch { condition: comparison };
        compared.push(point(12, 0, CfgPointKind::Read { binding: formal }, &[13]));
        compared.push(point(13, 0, CfgPointKind::Evaluate(integer), &[4]));
        facts.literal_array_unit = Some(LiteralArrayUnit {
            operands: Some([CfgPointId(12), CfgPointId(13)]),
            ..unit
        });
        let check_comparison = |points: &[ReferenceFlowPoint]| {
            let mut observed = AggregateFlowResult::default();
            observe(&facts, points, &[0, 1, 2, 3, 12, 13, 4, 5, 6, 7, 8, 9, 10, 11], &mut observed);
            observe(&facts, points, &[0, 1, 2, 3, 12, 13, 4, 5, 10, 11], &mut observed);
            observed.proved_literal_array_route().is_some()
        };
        assert!(check_comparison(&compared));
        let CfgPointKind::Evaluate(ScalarExpression::Binary { operator, .. }) = &mut compared[4].kind
        else { unreachable!() };
        *operator = BinaryOperator::Add;
        assert!(!check_comparison(&compared));
        facts.literal_array_unit = Some(unit);

        let mut different_binding = points.clone();
        let CfgPointKind::Assign { target, .. } = &mut different_binding[8].kind else { unreachable!() };
        target.binding.declaration_span = ByteSpan::new(1, 2);
        target.declaration_span = target.binding.declaration_span;

        let mut wrong_edges = points.clone();
        wrong_edges[5].successors.swap(0, 1);

        let mut unexpected_effect = points.clone();
        unexpected_effect[4].kind = CfgPointKind::Bind { binding: binding.clone() };

        let mut wrong_root_exit = points.clone();
        wrong_root_exit[11].kind = CfgPointKind::Join;

        for changed in [different_binding, wrong_edges, unexpected_effect, wrong_root_exit] {
            let mut observed = AggregateFlowResult::default();
            observe(&facts, &changed, &writer, &mut observed);
            observe(&facts, &changed, &fallthrough, &mut observed);
            assert!(observed.literal_array_unknown);
            assert!(observed.proved_literal_array_route().is_none());
        }

        // In the explicit two-arm form, preserving the original requires
        // the exact no-effect scope edge, not a shortcut from the branch.
        let mut explicit = points.to_vec();
        explicit[9].successors = vec![CfgPointId(12)];
        explicit[10] = point(10, 2, CfgPointKind::ScopeEnter, &[11]);
        explicit[11] = point(11, 2, CfgPointKind::ScopeExit, &[12]);
        explicit.push(point(12, 0, CfgPointKind::Join, &[13]));
        explicit.push(point(13, 0, CfgPointKind::ScopeExit, &[]));
        facts.literal_array_unit = Some(LiteralArrayUnit {
            retained: Some(LiteralArrayRetained {
                entry: CfgPointId(10), exit: CfgPointId(11),
                block: BlockOrdinal(2), scope: ReferenceScopeId(2),
            }),
            join: CfgPointId(12),
            ..unit
        });
        facts.scope_exits = BTreeMap::from([(CfgPointId(13), pair.block)]);
        let writer = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 12, 13];
        let retained = [0, 1, 2, 3, 4, 5, 10, 11, 12, 13];
        let mut observed = AggregateFlowResult::default();
        observe(&facts, &explicit, &writer, &mut observed);
        assert!(observed.proved_literal_array_route().is_none());
        observe(&facts, &explicit, &retained, &mut observed);
        let Some(LiteralArrayRoute::Replacement { original, write, exit, block, unit }) =
            observed.proved_literal_array_route() else { panic!("missing explicit retained edge proof") };
        assert_eq!((exit, unit), (CfgPointId(13), facts.literal_array_unit));

        let mut plans = ClosedAggregates::default();
        enroll_literal_array_pair(&mut plans, LiteralArrayPair { original, write, exit, block, unit, ..pair },
            LiteralArrayParameters::default());
        let caller = &plans.bodies[&pair.caller];
        assert!(matches!(caller.literal_array, Some(ClosedLiteralArray::Replacement {
            unit: Some(ClosedLiteralArrayUnit { retained_block: Some(2), .. }), ..
        })));
        assert_eq!(caller.scope_exits, BTreeMap::from([(0, CfgPointId(13))]));
        assert!(caller.releases.is_empty() && caller.input_release.is_none());

        explicit[11].kind = CfgPointKind::Read { binding };
        let mut observed = AggregateFlowResult::default();
        observe(&facts, &explicit, &writer, &mut observed);
        observe(&facts, &explicit, &retained, &mut observed);
        assert!(observed.literal_array_unknown);
        assert!(observed.proved_literal_array_route().is_none());

        // Mirror the compiler's then-before-else construction order.
        let call = explicit[7].kind.clone();
        let mut assign = explicit[8].kind.clone();
        let CfgPointKind::Assign { rhs_point, .. } = &mut assign else { unreachable!() };
        *rhs_point = CfgPointId(9);
        explicit[5].successors = vec![CfgPointId(6), CfgPointId(8)];
        explicit[6] = point(6, 1, CfgPointKind::ScopeEnter, &[7]);
        explicit[7] = point(7, 1, CfgPointKind::ScopeExit, &[12]);
        explicit[8] = point(8, 2, CfgPointKind::ScopeEnter, &[9]);
        explicit[9] = point(9, 2, call, &[10]);
        explicit[10] = point(10, 2, assign, &[11]);
        explicit[11] = point(11, 2, CfgPointKind::ScopeExit, &[12]);
        facts.literal_array_write = Some(LiteralArrayWrite {
            rhs: CfgPointId(9), writer: CfgPointId(10), ..pair.write
        });
        let mirrored = LiteralArrayUnit {
            writer_side: ArrayWriterSide::Else,
            writer_entry: CfgPointId(8), writer_exit: CfgPointId(11),
            writer_block: BlockOrdinal(2), writer_scope: ReferenceScopeId(2),
            retained: Some(LiteralArrayRetained {
                entry: CfgPointId(6), exit: CfgPointId(7), block: BlockOrdinal(1), scope: ReferenceScopeId(1),
            }),
            join: CfgPointId(12),
            ..unit.expect("observed explicit conditional")
        };
        facts.literal_array_unit = Some(mirrored);
        let retained_then = [0, 1, 2, 3, 4, 5, 6, 7, 12, 13];
        let writer_else = [0, 1, 2, 3, 4, 5, 8, 9, 10, 11, 12, 13];
        let mut observed = AggregateFlowResult::default();
        observe(&facts, &explicit, &retained_then, &mut observed);
        assert!(observed.proved_literal_array_route().is_none());
        observe(&facts, &explicit, &writer_else, &mut observed);
        let Some(LiteralArrayRoute::Replacement { original, write, exit, block, unit }) =
            observed.proved_literal_array_route() else { panic!("missing mirrored ownership proof") };
        assert_eq!(unit, Some(mirrored));
        let mut plans = ClosedAggregates::default();
        enroll_literal_array_pair(&mut plans, LiteralArrayPair { original, write, exit, block, unit, ..pair },
            LiteralArrayParameters::default());
        let caller = &plans.bodies[&pair.caller];
        assert!(matches!(caller.literal_array, Some(ClosedLiteralArray::Replacement {
            unit: Some(ClosedLiteralArrayUnit {
                writer_side: ArrayWriterSide::Else, writer_block: 2, retained_block: Some(1), ..
            }), ..
        })));
        assert_eq!(caller.scope_exits, BTreeMap::from([(0, CfgPointId(13))]));
        assert!(caller.releases.is_empty() && caller.input_release.is_none());

        for (id, other_scope) in [
            (4, ReferenceScopeId(1)),
            (5, ReferenceScopeId(2)),
            (11, ReferenceScopeId(0)),
        ] {
            let mut wrong_scope = explicit.clone();
            wrong_scope[id].scope = other_scope;
            let mut observed = AggregateFlowResult::default();
            observe(&facts, &wrong_scope, &retained_then, &mut observed);
            observe(&facts, &wrong_scope, &writer_else, &mut observed);
            assert!(observed.literal_array_unknown);
            assert!(observed.proved_literal_array_route().is_none());
        }

        facts.literal_array_unit = Some(LiteralArrayUnit { writer_side: ArrayWriterSide::Then, ..mirrored });
        let mut observed = AggregateFlowResult::default();
        observe(&facts, &explicit, &retained_then, &mut observed);
        observe(&facts, &explicit, &writer_else, &mut observed);
        assert!(observed.literal_array_unknown);
        assert!(observed.proved_literal_array_route().is_none());

        let mut dual = points[..6].to_vec();
        dual[5].successors = vec![CfgPointId(6), CfgPointId(10)];
        let mut alternate_assign = points[8].kind.clone();
        let CfgPointKind::Assign { rhs_point, .. } = &mut alternate_assign else { unreachable!() };
        *rhs_point = CfgPointId(11);
        dual.extend([
            point(6, 1, CfgPointKind::ScopeEnter, &[7]),
            point(7, 1, points[7].kind.clone(), &[8]),
            point(8, 1, points[8].kind.clone(), &[9]),
            point(9, 1, CfgPointKind::ScopeExit, &[14]),
            point(10, 2, CfgPointKind::ScopeEnter, &[11]),
            point(11, 2, points[7].kind.clone(), &[12]),
            point(12, 2, alternate_assign, &[13]),
            point(13, 2, CfgPointKind::ScopeExit, &[14]),
            point(14, 0, CfgPointKind::Join, &[15]),
            point(15, 0, CfgPointKind::ScopeExit, &[]),
        ]);
        let other = LiteralArrayWrite {
            rhs: CfgPointId(11), writer: CfgPointId(12),
            producer: OccurrenceOrdinal(7), destination: OccurrenceOrdinal(8), argument: None,
        };
        let dual_unit = LiteralArrayUnit {
            retained: Some(LiteralArrayRetained {
                entry: CfgPointId(10), exit: CfgPointId(13),
                block: BlockOrdinal(2), scope: ReferenceScopeId(2),
            }),
            other_write: Some(other),
            join: CfgPointId(14),
            writer_side: ArrayWriterSide::Then,
            writer_entry: CfgPointId(6), writer_exit: CfgPointId(9),
            writer_block: BlockOrdinal(1), writer_scope: ReferenceScopeId(1),
            ..mirrored
        };
        facts.literal_array_write = Some(pair.write);
        facts.literal_array_unit = Some(dual_unit);
        facts.scope_exits = BTreeMap::from([(CfgPointId(15), pair.block)]);
        let writer_then = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 14, 15];
        let writer_else = [0, 1, 2, 3, 4, 5, 10, 11, 12, 13, 14, 15];
        let mut observed = AggregateFlowResult::default();
        observe(&facts, &dual, &writer_then, &mut observed);
        assert!(observed.proved_literal_array_route().is_none());
        observe(&facts, &dual, &writer_else, &mut observed);
        let Some(LiteralArrayRoute::Replacement { original, write, exit, block, unit }) =
            observed.proved_literal_array_route() else { panic!("missing two writer proof") };
        let mut plans = ClosedAggregates::default();
        enroll_literal_array_pair(&mut plans, LiteralArrayPair { original, write, exit, block, unit, ..pair },
            LiteralArrayParameters::default());
        let caller = &plans.bodies[&pair.caller];
        assert!(matches!(caller.literal_array, Some(ClosedLiteralArray::Replacement {
            unit: Some(ClosedLiteralArrayUnit { other_writer: Some(ClosedLiteralArrayWrite { call: 7, destination: 8, .. }), .. }), ..
        })));
        assert_eq!(caller.scope_exits, BTreeMap::from([(0, CfgPointId(15))]));
        assert!(caller.releases.is_empty() && caller.input_release.is_none());

        let mut crossed = dual.clone();
        crossed[7].successors = vec![CfgPointId(12)];
        let crossed_then = [0, 1, 2, 3, 4, 5, 6, 7, 12, 13, 14, 15];
        let mut observed = AggregateFlowResult::default();
        observe(&facts, &crossed, &crossed_then, &mut observed);
        observe(&facts, &crossed, &writer_else, &mut observed);
        assert!(observed.literal_array_unknown);
        assert!(observed.proved_literal_array_route().is_none());

        facts.literal_array_unit = Some(LiteralArrayUnit {
            other_write: Some(LiteralArrayWrite { writer: CfgPointId(8), ..other }), ..dual_unit
        });
        let mut observed = AggregateFlowResult::default();
        observe(&facts, &dual, &writer_then, &mut observed);
        observe(&facts, &dual, &writer_else, &mut observed);
        assert!(observed.literal_array_unknown);
        assert!(observed.proved_literal_array_route().is_none());
    }

    #[test]
    fn checked_array_join_requires_both_exact_owner_edges() {
        use LiteralArrayPhase::*;
        let unit = LiteralArrayUnit {
            expression: OccurrenceOrdinal(4),
            condition: CfgPointId(3),
            operands: None,
            branch: CfgPointId(4),
            writer_side: ArrayWriterSide::Then,
            writer_entry: CfgPointId(5),
            writer_exit: CfgPointId(8),
            writer_block: BlockOrdinal(1),
            writer_scope: ReferenceScopeId(1),
            retained: None,
            other_write: None,
            join: CfgPointId(9),
        };
        let mut observed = LiteralArrayJoin::default();
        let preserved = unit.arrive_at_join(unit.join, Forked(unit.branch));
        assert!(preserved == JoinedOriginal);
        observed.observe_exit(preserved);
        assert!(!observed.complete(false));

        for other in [
            unit.arrive_at_join(unit.join, WriterExit(unit.branch)),
            unit.arrive_at_join(unit.join, Forked(unit.writer_exit)),
            unit.arrive_at_join(unit.writer_exit, WriterExit(unit.writer_exit)),
            unit.arrive_at_join(unit.join, Replaced),
        ] {
            assert!(other == Unknown);
            observed.observe_exit(other);
            assert!(!observed.complete(false));
        }

        let replaced = unit.arrive_at_join(unit.join, WriterExit(unit.writer_exit));
        assert!(replaced == JoinedFresh);
        observed.observe_exit(replaced);
        assert!(observed.complete(false));
    }

    #[test]
    fn recursive_array_tail_needs_a_grounded_formal_path() {
        let body = (0, 1, None);
        let call = CfgPointId(5);
        let tail = || Some(ArrayReturnPath {
            dependencies: BTreeSet::from([call]),
            tail_call: Some(call),
            last_producer: Some(call),
        });
        let base = || Some(ArrayReturnPath {
            dependencies: BTreeSet::new(),
            tail_call: None,
            last_producer: None,
        });
        let resolve_self = |point| (point == call).then_some(body);

        assert_eq!(
            grounded_array_return_dependencies(body, &BTreeSet::from([base(), tail()]), resolve_self),
            Some(BTreeSet::from([body])),
        );
        assert_eq!(
            grounded_array_return_dependencies(body, &BTreeSet::from([tail()]), resolve_self),
            None,
        );
        assert_eq!(
            grounded_array_return_dependencies(body, &BTreeSet::from([base(), tail(), None]), resolve_self),
            None,
        );
        assert_eq!(
            grounded_array_return_dependencies(body, &BTreeSet::from([base(), tail()]), |_| None),
            None,
        );
        let unlinked_tail = Some(ArrayReturnPath {
            dependencies: BTreeSet::new(),
            tail_call: Some(call),
            last_producer: Some(call),
        });
        assert_eq!(
            grounded_array_return_dependencies(body, &BTreeSet::from([base(), unlinked_tail]), resolve_self),
            None,
        );
    }

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
