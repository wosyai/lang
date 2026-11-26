//! The read side of the exact selected/predecessor SQLite generation boundary.
use super::{
    load_usage, read_selection, ClientSourceLimits, Closure, Files, Result, SourceAttempt,
    SqliteSourceState, StoredNodes,
};
use rusqlite::{Connection, TransactionBehavior};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use wosy_protocol::{
    BaseCstProposal, CstAddition, CstDelta, Node, NodeId,
    SourceIndexAddition, BASE_CST_DELTA_VERSION,
};

impl SqliteSourceState {
    /// Construct a sparse proposal for the exact still-pending base source
    /// attempt, without observing live source, parsing, or acknowledging it.
    /// Reads both bounded source closures from one local snapshot, keeping
    /// base identities and novel encoded records, not two copies of the CST.
    /// A cold client needs the separate complete source identity handshake.
    pub fn pending_base_cst_proposal(
        &mut self, target: &str, attempt: SourceAttempt,
    ) -> Result<BaseCstProposal> {
        let tx = self.db.transaction_with_behavior(TransactionBehavior::Deferred)?;
        let selection = read_selection(&tx, target)?;
        if selection.pending != Some(attempt) {
            return Err("base CST proposal is not the current source attempt".into());
        }
        let base = selection.selected.ok_or("base CST proposal requires a selected predecessor")?;
        let root = attempt.root();
        let (known, previous) =
            trace_and_index(&tx, &self.project, target, base, self.limits, |_, _| ())?;
        let mut novel = BTreeMap::new();
        let (_, proposed) =
            trace_and_index(&tx, &self.project, target, root, self.limits, |id, bytes| {
                if !known.contains(id) { novel.insert(id, bytes); }
            })?;
        let mut files = Vec::new();
        for key in previous.keys().chain(proposed.keys()).collect::<BTreeSet<_>>() {
            let old_file_root = previous.get(key).map(|file| file.file);
            let current = proposed.get(key);
            let new_file_root = current.map(|file| file.file);
            if old_file_root == new_file_root { continue; }
            let mut added = Vec::new();
            if let Some(file) = current {
                let mut pending = vec![file.executable, file.authored];
                while let Some(id) = pending.pop() {
                    // Absent from novel means reachable from the selected
                    // base, or already emitted by this or an earlier file.
                    let Some(bytes) = novel.remove(&id) else { continue; };
                    pending.extend(Node::child_ids(&bytes).ok_or("invalid persisted CST node")?);
                    added.push(CstAddition::from_canonical(id, bytes)?);
                }
            }
            files.push(CstDelta { file: key.clone(), old_file_root, new_file_root, added });
        }
        let source_index = novel.into_iter().map(|(id, bytes)| SourceIndexAddition::from_canonical(id, bytes))
            .collect::<std::result::Result<_, _>>()?;
        tx.commit()?;
        Ok(BaseCstProposal { version: BASE_CST_DELTA_VERSION, base, root, files, source_index })
    }
}


fn trace_and_index(
    tx: &Connection, project: &Path, target: &str, root: NodeId, limits: ClientSourceLimits,
    keep: impl FnMut(NodeId, Vec<u8>),
) -> Result<(Closure, Files)> {
    if let Some(usage) = load_usage(tx, root)? { usage.admit(limits)?; }
    let mut read = StoredNodes::new(tx)?;
    // A usage receipt can refuse early. Verify the actual closure before
    // retaining the file index even when a migrated root has no receipt.
    let closure = read.trace(root, limits, keep)?;
    let (config, entries) = SqliteSourceState::header(project, &mut read, target, root)?;
    Ok((closure, read.file_index(&config, entries)?))
}
