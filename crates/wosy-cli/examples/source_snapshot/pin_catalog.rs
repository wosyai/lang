use super::{milliseconds, source_reconciliation::admission_limits, Result};
use std::{cell::Cell, num::NonZeroUsize, path::Path, sync::Arc, time::Instant};
use wosy_cli::{
    source_recovery::ClientSourceLimits,
    source_state::{PinPage, PinPageResult, PinSyncStatus, ReplaySource, SqliteSourceState},
};
use wosy_daemon::{SourceStorageWorker, SourceStore};
use wosy_protocol::{SourcePinReflection, SourcePinRetention};

const DIRECT_NAME: &str = "40-direct-collision";
const INSERTED_BEFORE_CURSOR: &str = "00-inserted-before-cursor";
const INSERTED_DURING_PAGE: &str = "01-inserted-during-page";
const LIMITS: ClientSourceLimits = ClientSourceLimits { nodes: 25_000, encoded_bytes: 16 << 20 };

fn page(result: PinPageResult) -> Result<PinPage> {
    match result {
        PinPageResult::Page(page) => Ok(page),
        PinPageResult::RestartRequired => Err("fixture catalog unexpectedly changed".into()),
    }
}

async fn sweep(
    local: &mut SqliteSourceState, project: &Path, worker: &SourceStorageWorker, size: NonZeroUsize,
) -> Result<(usize, usize, Vec<serde_json::Value>)> {
    let limits = admission_limits(false);
    let mut cursor = local.begin_source_pin_scan()?;
    let mut response = Vec::new();
    let mut pages = 0;
    let mut empty_pages = 0;
    loop {
        let found = page(local.reconcile_source_pin_page(&cursor, size,
            |intent, replay| worker.reflect_source_pin(intent, replay, limits),
        ).await?)?;
        assert!(found.items.len() <= size.get());
        pages += 1;
        if found.items.is_empty() { empty_pages += 1; }
        for row in found.items {
            let (status, replayed) = match row.status {
                PinSyncStatus::Durable { sent_replay } => ("durable", sent_replay),
                PinSyncStatus::NameHeldByDirectPin => ("direct pin", false),
                _ => return Err("unexpected fixture reflection status".into()),
            };
            response.push(serde_json::json!({
                "name": row.intent.name, "root": row.intent.source.root.hex(), "status": status,
                "retained": row.intent.retention == SourcePinRetention::Retained, "replayed": replayed,
            }));
        }
        let Some(next) = found.continuation else { break };
        cursor = next;
        // Exercise a genuine SQLite close/reopen between pages.
        *local = SqliteSourceState::open(project, LIMITS)?;
    }
    Ok((pages, empty_pages, response))
}

pub(super) async fn run(
    local: &mut SqliteSourceState, project: &Path, path: &Path, target: &str, collision: bool,
) -> Result<serde_json::Value> {
    let store = Arc::new(SourceStore::open(path, 8 << 20)?);
    let current = local.selection(target)?.selected.ok_or("fixture has no selected source")?;
    if collision {
        let selected = local.replay(target, ReplaySource::Selected)?.ok_or("fixture has no selected source replay")?;
        store.publish(current, &selected.source.delta).await?;
        store.pin_source(current, DIRECT_NAME).await?;
    }
    let bound = |n| NonZeroUsize::new(n).unwrap();
    let worker = SourceStorageWorker::new(
        Arc::clone(&store), tokio::runtime::Handle::current(), bound(1), bound(2), bound(20_000),
    )?;
    let size = bound(2);
    let started = Instant::now();
    let (pages, empty_pages, rows) = sweep(local, project, &worker, size).await?;
    let first_scan_ms = milliseconds(started);
    let (stale_rescanned, stopped_during_page, after_changes) = if collision {
        let direct = store.source_pins(current)?.into_iter()
            .find(|pin| pin.name == DIRECT_NAME).ok_or("fixture direct pin disappeared")?;
        assert_eq!(direct.root, current);
        (false, false, Vec::new())
    } else {
        let limits = admission_limits(false);
        let first = local.begin_source_pin_scan()?;
        let old = page(local.reconcile_source_pin_page(&first, size,
            |intent, replay| worker.reflect_source_pin(intent, replay, limits),
        ).await?)?.continuation.ok_or("fixture needs a second page")?;
        let mut writer = SqliteSourceState::open(project, LIMITS)?;
        writer.pin_selected_source(target, INSERTED_BEFORE_CURSOR)?;
        let calls = Cell::new(0);
        let skipped = local.reconcile_source_pin_page(&old, size, |intent, replay| {
            calls.set(calls.get() + 1);
            worker.reflect_source_pin(intent, replay, limits)
        }).await?;
        assert!(matches!(skipped, PinPageResult::RestartRequired));
        assert_eq!(calls.get(), 0);

        let first = local.begin_source_pin_scan()?;
        let calls = Cell::new(0);
        let during = local.reconcile_source_pin_page(&first, size, |intent, replay| {
            let call = calls.get() + 1;
            calls.set(call);
            let worker = &worker;
            async move {
                let result = worker.reflect_source_pin(intent, replay, limits).await?;
                if call == 1 {
                    assert!(matches!(result, SourcePinReflection::Durable(_)));
                    SqliteSourceState::open(project, LIMITS)?.pin_selected_source(target, INSERTED_DURING_PAGE)?;
                }
                Ok(result)
            }
        }).await?;
        assert!(matches!(during, PinPageResult::RestartRequired));
        assert_eq!(calls.get(), 1);
        let (_, _, final_rows) = sweep(local, project, &worker, size).await?;
        assert_eq!(final_rows[0]["name"], INSERTED_BEFORE_CURSOR);
        assert_eq!(final_rows[1]["name"], INSERTED_DURING_PAGE);
        (true, true, final_rows)
    };
    drop(worker);
    Arc::try_unwrap(store).ok().expect("fixture owns its catalog").close().await?;
    Ok(serde_json::json!({
        "rows": rows, "pages": pages, "empty_pages": empty_pages, "elapsed_ms": first_scan_ms,
        "stale_cursor_rescanned": stale_rescanned, "stopped_during_page": stopped_during_page, "after_changes": after_changes,
    }))
}
