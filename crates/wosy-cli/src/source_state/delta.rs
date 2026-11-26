//! The read side of the exact selected/predecessor SQLite generation boundary.
use super::{
    load_usage, read_selection, ClientSourceLimits, Closure, Files, Result, SourceAttempt,
    SqliteSourceState, StoredNodes,
};
use rusqlite::{Connection, TransactionBehavior};
use std::collections::BTreeMap;
use std::path::Path;
use wosy_protocol::{
    partition_cst_files, BaseCstProposal, BaseCstProposalBinding, BuildSourceSelection, CstAddition, CstDelta, NodeId,
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
    ) -> Result<(BaseCstProposalBinding, BaseCstProposal)> {
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
        let partitions = partition_cst_files(&previous, &proposed, |id| novel.get(&id).map(Vec::as_slice))?;
        let files = partitions.into_iter().map(|partition| {
            let added = partition.added.into_iter().map(|id| {
                let bytes = novel.remove(&id).expect("partition only contains once-assigned novel records");
                CstAddition::from_canonical(id, bytes)
            }).collect::<std::result::Result<_, _>>()?;
            Ok(CstDelta {
                file: partition.file, old_file_root: partition.old_file_root,
                new_file_root: partition.new_file_root, added,
            })
        }).collect::<std::result::Result<_, &'static str>>()?;
        let source_index = novel.into_iter().map(|(id, bytes)| SourceIndexAddition::from_canonical(id, bytes))
            .collect::<std::result::Result<_, _>>()?;
        let project = self.project.to_str().ok_or("workspace path is not UTF-8")?;
        let binding = BaseCstProposalBinding {
            selected: BuildSourceSelection { root: base, project: project.into(), target: target.into() },
            proposed: root, attempt: attempt.generation(),
        };
        tx.commit()?;
        Ok((binding, BaseCstProposal {
            version: BASE_CST_DELTA_VERSION, base, root, attempt: attempt.generation(), files, source_index,
        }))
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
