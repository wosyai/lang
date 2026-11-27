pub(super) const MAX_ACTUAL_CALL_DEPTH: usize = 64;
pub(super) const MAX_PARTITION_TRANSFER_WORK: usize = MAX_ACTUAL_CALL_DEPTH * MAX_ACTUAL_CALL_DEPTH;

// Hold a sticky exhausted value so argument discovery can distinguish a
// missing operand from a refused scan even on the unary fallback route.
pub(super) fn charge_call_work(work: &mut usize, cost: usize) -> Option<()> {
    *work = work.saturating_add(cost);
    (*work <= MAX_PARTITION_TRANSFER_WORK).then_some(())
}
