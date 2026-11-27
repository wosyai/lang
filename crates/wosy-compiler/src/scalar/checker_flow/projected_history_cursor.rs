//! Suspensible projected graph execution. The parent keeps its exact block/row
//! cursor while the child runs; only the waiting call's return partitions move.
use super::*;
use std::{cell::{Cell, RefCell}, collections::VecDeque};

type Target = (usize, usize);
struct Pending<'a> {
    effect: ExactCallEffect<'a>,
    row: usize,
    receipts: VecDeque<CallableHistoryCall>,
}
enum Machine<'a> {
    Unit { roots: IntegerRoots<'a>, block: IntegerBlockCursor },
    Scalar { cursor: ExactCallCursor<'a>, inputs: Vec<Rc<IntegerSet>> },
}
struct Frame<'a> {
    target: Target,
    machine: Machine<'a>,
    pending: Option<Pending<'a>>,
}
impl<'a> Machine<'a> {
    fn parts(&mut self) -> (&mut IntegerRoots<'a>, &mut IntegerBlockCursor) {
        match self {
            Self::Unit { roots, block } => (roots, block),
            Self::Scalar { cursor, .. } => (&mut cursor.roots, &mut cursor.block),
        }
    }
    fn enter(body: &'a FunctionCallSites, tracker: &'a RefCell<CallableHistories<'a>>, entry: Option<CallableHistoryEntry>) -> Option<Self> {
        tracker.borrow().charge(body.occurrences.len().saturating_add(body.boundary.parameters.len()))?;
        if !body.checker_returns_unit() {
            let [result] = body.boundary.results.as_slice() else { return None; };
            let CallableHistoryEntry(mut row) = entry?;
            let inputs = row.cells.clone();
            let mut cursor = match inputs.as_slice() {
                [_] => ExactCallCursor::with_row(body, &result.ty, row, Some(0))?,
                [_, _] => {
                    row.projected_from = 1;
                    ExactCallCursor::with_rows(body, &result.ty, IntegerInputs(vec![row]), None, Some([0,1]))?
                }
                _ => return None,
            };
            cursor.roots.callable_history = Some(tracker);
            return Some(Self::Scalar { cursor, inputs });
        }
        let ids = body.boundary.checker_parameter_bindings.as_deref()?;
        if ids.len() != body.boundary.parameters.len() { return None; }
        let snapshots = !body.structure.checker_selected_snapshots.is_empty();
        let mut roots = IntegerRoots { callable_history: Some(tracker), allow_snapshot_history: snapshots,
            allow_ordinary_affine_overwrites: snapshots,
            allow_ordinary_contextual_places: !snapshots && body.structure.has_ordinary_contextual_places(),
            ..IntegerRoots::default() };
        for (id, formal) in ids.iter().zip(&body.boundary.parameters) {
            if !matches!(formal.ty, ScalarType::F32 | ScalarType::F64) { roots.add(id, &formal.ty); }
        }
        roots.parameters = roots.axes.len();
        let row = match entry {
            Some(CallableHistoryEntry(row)) if row.len() == roots.parameters && row.iter().all(|d| !d.is_empty()) => row,
            Some(_) => return None,
            None => roots.axes.iter().map(|(_, ty)| ordinary_parameter_set(ty, body.checker_has_authored_match_history()).map(Rc::new)).collect::<Option<IntegerRow>>()?,
        };
        let block = IntegerBlockCursor::new(body.structure.root, roots.parameters, IntegerInputs(vec![row]));
        Some(Self::Unit { roots, block })
    }
}

enum Outcome<'a> { Park(ExactCallEffect<'a>), Return(CallableHistoryCall) }

pub(in super::super) fn drive_finite_history<'p>(
    root: Target, work: &Cell<usize>, observed: &Cell<bool>,
    body_of: impl Fn(Target) -> Option<&'p FunctionCallSites>,
    resolve: impl Fn(Target, &FunctionCallSites, OccurrenceOrdinal, &CheckedCallSignature) -> Option<Target>,
    mut record: impl FnMut(Target, IntegerCheckFailure<'_>, Vec<CallableHistoryStep>) -> Option<bool>,
) -> Option<CallableHistoryCompletion> {
    let histories = RefCell::new(CallableHistories {
        shared: true, work, nodes: Vec::new(), interned: HashMap::new(), step_owner: None, stopped: false,
        outgoing_call: None, scalar_return_steps: None, unit_return_arity: None, unit_return_paths: None, received_unit_paths: None,
    });
    let mut stack = vec![Frame { target: root, machine: Machine::enter(body_of(root)?, &histories, None)?, pending: None }];
    loop {
        let frame = stack.last_mut()?;
        let target = frame.target;
        let body = body_of(target)?;
        histories.borrow_mut().step_owner = (target.0 != root.0).then_some(target.0);
        let mut ready = VecDeque::new();
        let mut ready_site = None;
        if let Some(pending) = &mut frame.pending {
            let site = pending.effect.occurrence();
            let signature = body.checked_call_signatures.get(&site)?;
            let (roots, cursor) = frame.machine.parts();
            if let Some(row) = cursor.path.0.get(pending.row) {
                let span = body.occurrences.get(site.0)?.span;
                histories.borrow().charge(row.len().saturating_add(2))?;
                let mut path = IntegerInputs(vec![row.clone()]);
                let mut tracker = histories.borrow_mut();
                let mark = tracker.boundary(span, false);
                tracker.append(path.0.first_mut()?, mark);
                let mut fuel = work.get();
                let mut entry = match pending.effect {
                    ExactCallEffect::Unit(_) => {
                        let operands = projected_unit_operands(body, site, signature, roots, &path, &mut fuel)?;
                        work.set(fuel);
                        ProjectedUnitHistory { operands: &operands, row: path.0.first()? }.entry(&signature.parameters, work)?
                    }
                    ExactCallEffect::Scalar { binding: Some(_), .. } => {
                        let arg = match signature.parameters.len() {
                            1 => actual_nested_argument(body, site, roots, &path, true, false, None),
                            2 => nested_pair_argument(body, site, roots, &path, true, false, true, false, None)?,
                            _ => return None,
                        };
                        arg.callable_history_entry(&signature.parameters, work)?
                    }
                    _ => return None,
                };
                entry.0.callable_history = path.0.first()?.callable_history;
                drop(tracker);
                let child = resolve(target, body, site, signature)?;
                pending.row += 1;
                stack.push(Frame { target: child, machine: Machine::enter(body_of(child)?, &histories, Some(entry))?, pending: None });
                continue;
            }
            if pending.row != cursor.path.0.len() || pending.receipts.len() != pending.row { return None; }
            // The call item was parked before evaluating a row or emitting an effect.
            // Visit that item once with the now-owned receipts, then continue normally.
            cursor.next = cursor.next.checked_sub(1)?;
            roots.ready_history_call = Some(site);
            ready_site = Some(site);
            ready = std::mem::take(&mut pending.receipts);
            frame.pending = None;
        }
        let mut apply = |_: &FunctionCallSites, site, _: &CheckedCallSignature, _: NestedIntegerArgument<'_>, ty: &ScalarType| {
            if Some(site) != ready_site { return None; }
            accept_callable_history_return(&histories, ready.pop_front()?, ty)
        };
        let mut unpublishable = false;
        let mut sink = |mut failure: IntegerCheckFailure<'_>| {
            let mut tracker = histories.borrow_mut();
            let accepted = (|| {
                let node = failure.state.0.first()?.callable_history;
                tracker.charge(failure.state.0.len().saturating_add(failure.axes.len()))?;
                for row in &mut failure.state.0 {
                    if row.callable_history != node { return None; }
                    row.callable_history = 0;
                }
                record(target, failure, tracker.steps(node)?)
            })();
            match accepted { Some(true) => tracker.stopped = true, Some(false) => {},
                None => { unpublishable = true; tracker.stopped = true; } }
        };
        let outcome = match &mut frame.machine {
            Machine::Unit { roots, block } => {
                let mut failed = IntegerFailureCollection { continuation: IntegerRequirementContinuation::Empty,
                    projected: None, allow_projected_calls: true, allow_contextual_local_transfers: false, sink: &mut sink };
                let progress = drive_integer_block::<false, true, _, false>(body, roots, block, None, None,
                    &mut Some(&mut apply), &mut Some(&mut failed), &mut &mut [][..], (), false, None, None, None);
                if unpublishable { return None; }
                if histories.borrow().stopped { return Some(CallableHistoryCompletion::LimitReached); }
                match progress? {
                    IntegerControl::PendingCall(effect) => Outcome::Park(effect),
                    IntegerControl::Continue(path) => {
                        observed.set(observed.get() || roots.reached_obligation);
                        let mut pieces = Vec::with_capacity(path.0.len());
                        for row in path.0 {
                            let domains = row.cells.get(..roots.parameters)?;
                            histories.borrow().charge(domains.iter().map(|d| 1+d.interval_count()).sum())?;
                            pieces.push(UnitHistoryPiece { domains: domains.to_vec(), steps: ReturnHistory::Shared(row.callable_history) });
                        }
                        Outcome::Return(CallableHistoryCall::UnitPartitions(pieces))
                    }
                    _ => return None,
                }
            }
            Machine::Scalar { cursor, .. } => {
                match cursor.advance::<false>(body, Some(&mut apply), false, false, false, false, &mut sink)? {
                    ExactCallProgress::Pending(effect) => Outcome::Park(effect),
                    ExactCallProgress::Done { active: false, result } => {
                        let Machine::Scalar { cursor, inputs } = stack.pop()?.machine else { return None; };
                        if !ready.is_empty() { return None; }
                        let receipt = finish_closed_scalar_history(cursor, inputs, result, &histories)?;
                        stack.last_mut()?.pending.as_mut()?.receipts.push_back(receipt);
                        continue;
                    }
                    _ => return None,
                }
            }
        };
        if !ready.is_empty() || frame.machine.parts().0.ready_history_call.is_some() { return None; }
        match outcome {
            Outcome::Park(effect) => frame.pending = Some(Pending { effect, row: 0, receipts: VecDeque::new() }),
            Outcome::Return(receipt) => {
                stack.pop()?;
                if let Some(parent) = stack.last_mut() { parent.pending.as_mut()?.receipts.push_back(receipt); }
                else { return Some(CallableHistoryCompletion::Exhausted(observed.get())); }
            }
        }
    }
}
