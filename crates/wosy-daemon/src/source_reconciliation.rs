//! Restore already-acknowledged source after loss of recomputable store data.
use super::{
    check_source_marker, key, read_verified_node_with,
    source_proposal::{matches_source_scope, stage_reconciled_closure, take},
    storage_worker::QueryWorker,
    BaseCstAdmissionLimits, PublishReport, Result, SourceStorageWorker, SourceStore, SNAPSHOT_ROOTS,
};
use std::sync::Arc;
use surrealkv::{Durability, Mode, Transaction};
use wosy_protocol::{source_preparation_parts, BuildSourceSelection, Delta, Kind, Node, NodeId, SourceConfig,
    SourcePreparationParts};

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
        self.spawn_reserved_query(slot, None, move |worker| async move {
            reconcile_on_worker(&worker, Arc::new(selected), replay, limits).await
        }).await?
    }
}

pub(super) async fn reconcile_on_worker(
    worker: &QueryWorker, selected: Arc<BuildSourceSelection>, replay: Option<Delta>,
    limits: BaseCstAdmissionLimits,
) -> Result<SourceReconciliation> {
    let root = selected.root;
    let replay = replay.map(Arc::new);
    // Keep the existing slot across blocking validation and async commits.
    // At most one retry; each attempt uses a new transaction cut.
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
        check_source_marker(root, &published)?;
        if !matches_source_scope(&checked_published_source_header(&tx, root)?, selected) {
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

pub(super) fn checked_published_source_header(tx: &Transaction, root: NodeId) -> Result<SourceConfig> {
    Ok(checked_published_source_header_with_top(tx, root)?.1)
}

pub(super) fn checked_published_source_header_with_top(tx: &Transaction, root: NodeId) -> Result<(Node, SourceConfig)> {
    checked_source_header_using(root, |id| read_verified_node_with(tx, id, |_| Ok(())))
}

pub(super) fn checked_source_header_using(
    root: NodeId, mut read: impl FnMut(NodeId) -> Result<Node>,
) -> Result<(Node, SourceConfig)> {
    let source = read(root)?;
    let parts = source_preparation_parts(&source).ok_or("unsupported acknowledged source schema")?;
    let config = checked_source_config_for_parts(&parts, read(parts.config)?)?;
    Ok((source, config))
}

pub(super) fn checked_source_config_for_parts(parts: &SourcePreparationParts<'_>, config: Node) -> Result<SourceConfig> {
    if config.kind != Kind::Config { return Err("invalid acknowledged source configuration kind".into()); }
    super::validate_shape(&config)?;
    let config: SourceConfig = serde_json::from_slice(&config.data)?;
    if !parts.matches_config(&config) {
        return Err("invalid acknowledged source configuration schema".into());
    }
    Ok(config)
}

pub(super) fn bounded_source_records(
    tx: &Transaction, limits: BaseCstAdmissionLimits,
) -> impl FnMut(NodeId) -> Result<Node> + '_ {
    let (mut records, mut encoded) = (limits.records, limits.record_bytes);
    move |id| read_verified_node_with(tx, id, |bytes| {
        take(&mut records, 1, "source index exceeds the record budget")?;
        take(&mut encoded, bytes, "source index exceeds the byte budget")
    })
}
