pub(super) const MAX_ACTUAL_CALL_DEPTH: usize = 64;

#[cfg(test)]
pub(super) const FORMER_TRANSFER_CEILING: usize = 4096;

// Analysis work is telemetry, not part of the language's proof outcome.
// Saturation preserves the count without refusing valid later work.
pub(super) fn charge_call_work(work: &mut usize, cost: usize) -> Option<()> {
    *work = work.saturating_add(cost);
    Some(())
}
