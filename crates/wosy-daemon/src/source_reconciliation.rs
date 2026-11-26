//! Restore already-acknowledged source after loss of recomputable store data.
use super::{
    key,
    source_proposal::{matches_source_scope, stage_reconciled_closure},
    BaseCstAdmissionLimits, PublishReport, Result, SourceStorageWorker, SourceStore, NODES, SNAPSHOT_ROOTS,
};
use std::sync::Arc;
use surrealkv::{Durability, Mode, Transaction};
use wosy_protocol::{source_preparation_parts, BuildSourceSelection, Delta, Kind, Node, NodeId, SourceConfig};

pub enum SourceReconciliation {
    Known { root: NodeId },
    ReplayRequired { root: NodeId },
    Restored { root: NodeId, report: PublishReport, validated_files: usize, conflicts: usize },
}

enum Plan {
    Known,
    ReplayRequired,
    Write { transaction: Transaction, report: PublishReport, validated_files: usize },
}

impl SourceStorageWorker {
    /// Only a trusted local adapter may present an acknowledged workspace and
    /// target. Probe with no payload first; never read or synthesize live
    /// source for replay. This is source-only, not a Build/Check or SnapshotRoot
    /// result, a semantic readiness signal, or a wire-level request.
    /// Dropping the observer does not cancel already-admitted source work.
    pub async fn reconcile_acknowledged_source(
        &self, selected: BuildSourceSelection, replay: Option<Delta>, limits: BaseCstAdmissionLimits,
    ) -> Result<SourceReconciliation> {
        let slot = self.try_slot()?;
        self.spawn_reserved_query(slot, move |worker| async move {
            let root = selected.root;
            let selected = Arc::new(selected);
            let replay = replay.map(Arc::new);
            // Keep the existing slot across blocking validation and async
            // commits. A second conflict is returned instead of restarting
            // validation indefinitely; each attempt starts in a new cut.
            for conflicts in 0..=1 {
                let selection = Arc::clone(&selected);
                let payload = replay.clone();
                let plan = worker.phase(None, move |store| {
                    reconciliation_plan(store, &selection, payload.as_deref(), limits)
                }).await?;
                let (mut transaction, report, validated_files) = match plan {
                    Plan::Known => return Ok(SourceReconciliation::Known { root }),
                    Plan::ReplayRequired => return Ok(SourceReconciliation::ReplayRequired { root }),
                    Plan::Write { transaction, report, validated_files } => (transaction, report, validated_files),
                };
                match transaction.commit().await {
                    Ok(()) => return Ok(SourceReconciliation::Restored { root, report, validated_files, conflicts }),
                    Err(surrealkv::Error::TransactionWriteConflict) if conflicts == 0 => {}
                    Err(error) => return Err(error.into()),
                }
            }
            unreachable!("a second commit conflict is returned")
        }).await?
    }
}

fn reconciliation_plan(
    store: &SourceStore, selected: &BuildSourceSelection, replay: Option<&Delta>,
    limits: BaseCstAdmissionLimits,
) -> Result<Plan> {
    let root = selected.root;
    let marker = key(SNAPSHOT_ROOTS, root);
    let mut tx = store.tree.begin_with_mode(if replay.is_some() { Mode::ReadWrite } else { Mode::ReadOnly })?;
    let published = if replay.is_some() { tx.get_for_update(&marker)? } else { tx.get(&marker)? };
    if let Some(published) = published {
        if published.as_slice() != root.0 { return Err("invalid acknowledged source marker".into()); }
        let source = checked_header_node(&tx, root)?;
        let parts = source_preparation_parts(&source).ok_or("unsupported acknowledged source schema")?;
        let config = checked_header_node(&tx, parts.config)?;
        if config.kind != Kind::Config { return Err("invalid acknowledged source configuration kind".into()); }
        let config: SourceConfig = serde_json::from_slice(&config.data)?;
        if source.data != config.schema()
            || config.parser.schema.0 != *wosy_syntax::base_cst_schema()
            || !matches_source_scope(&config, selected)
        {
            return Err("published source does not match the acknowledged workspace and target".into());
        }
        return Ok(Plan::Known);
    }
    let Some(replay) = replay else { return Ok(Plan::ReplayRequired) };
    tx.set_durability(Durability::Eventual);
    let mut report = PublishReport::default();
    let validated_files = stage_reconciled_closure(&mut tx, selected, replay, limits, &mut report)?;
    tx.set(&marker, &root.0)?;
    Ok(Plan::Write { transaction: tx, report, validated_files })
}

fn checked_header_node(tx: &Transaction, id: NodeId) -> Result<Node> {
    let bytes = tx.get(&key(NODES, id))?.ok_or("acknowledged source header is missing")?;
    if Node::id(&bytes) != id { return Err("acknowledged source header digest mismatch".into()); }
    Node::decode(&bytes).ok_or_else(|| "invalid acknowledged source header".into())
}
