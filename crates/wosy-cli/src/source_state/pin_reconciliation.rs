//! Bounded, resumable reflection of named base-source pins.
use super::{pins::intent_from_row, Result, SqliteSourceState};
use rusqlite::{params, Connection, TransactionBehavior};
use std::{future::Future, num::NonZeroUsize};
use wosy_protocol::{Delta, SourcePinIntent, SourcePinReflection};

const MAX_PAGE: usize = 64;
// Bound physical rows, not just eligible intents. Older databases can have
// any number of releases with no root witness. They count in the window.
const PIN_PAGE_SQL: &str = "SELECT name, target, COALESCE(root, released_root), generation, root IS NOT NULL
    FROM source_pins WHERE name > ?1 ORDER BY name LIMIT ?2";

#[derive(Clone, Debug)]
pub struct PinScanCursor {
    workspace: String,
    revision: i64,
    after: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PinSyncStatus {
    Durable { sent_replay: bool },
    Superseded,
    NameHeldByDirectPin,
    ReplayStillRequired,
}

pub struct PinPageItem {
    pub intent: SourcePinIntent,
    pub status: PinSyncStatus,
}

pub struct PinPage {
    pub items: Vec<PinPageItem>,
    /// None means this catalog revision has no more names to scan. Inspect
    /// each status; reaching the end does not make every pin durable.
    pub continuation: Option<PinScanCursor>,
}

pub enum PinPageResult {
    Page(PinPage),
    /// The local catalog changed. Previously committed daemon work stays
    /// committed; the caller must rescan the latest catalog from the start.
    RestartRequired,
}

pub(super) fn initialize(db: &Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE source_pin_catalog (
            singleton INTEGER PRIMARY KEY CHECK (singleton = 0),
            revision INTEGER NOT NULL CHECK (revision >= 0)
         ) STRICT;
         INSERT INTO source_pin_catalog (singleton, revision) VALUES (0, 0);
         PRAGMA main.user_version = 6;",
    )?;
    Ok(())
}

pub(super) fn catalog_revision(db: &Connection) -> Result<i64> {
    Ok(db.query_row("SELECT revision FROM source_pin_catalog WHERE singleton = 0", [], |row| row.get(0))?)
}

pub(super) fn advance_catalog(db: &Connection) -> Result<()> {
    if db.execute(
        "UPDATE source_pin_catalog SET revision = revision + 1
         WHERE singleton = 0 AND revision < 9223372036854775807", [],
    )? != 1 {
        return Err("local source pin catalog revision is unavailable or exhausted".into());
    }
    Ok(())
}

impl SqliteSourceState {
    /// Capture a cursor for this local workspace. It remains useful across
    /// a client reopen, provided no pin or release changed the catalog.
    pub fn begin_source_pin_scan(&self) -> Result<PinScanCursor> {
        let workspace = self.project.to_str().ok_or("workspace path is not UTF-8")?.to_owned();
        let revision = catalog_revision(&self.db)?;
        Ok(PinScanCursor { workspace, revision, after: String::new() })
    }

    /// Process one page with the trusted local daemon adapter. An immutable
    /// page is read before any callback, with no SQLite transaction held
    /// across daemon work. A local change discards the continuation without
    /// rolling back independent daemon commits.
    pub async fn reconcile_source_pin_page<F, Reply>(
        &mut self, cursor: &PinScanCursor, limit: NonZeroUsize, mut reflect: F,
    ) -> Result<PinPageResult>
    where
        F: FnMut(SourcePinIntent, Option<Delta>) -> Reply,
        Reply: Future<Output = Result<SourcePinReflection>>,
    {
        if self.project.to_str() != Some(cursor.workspace.as_str()) {
            return Err("source pin cursor belongs to another workspace".into());
        }
        let limit = limit.get().min(MAX_PAGE);
        let intents = {
            let tx = self.db.transaction_with_behavior(TransactionBehavior::Deferred)?;
            if catalog_revision(&tx)? != cursor.revision { return Ok(PinPageResult::RestartRequired); }
            let mut select = tx.prepare(PIN_PAGE_SQL)?;
            let rows = select.query_map(params![cursor.after, limit as i64 + 1],
                |row| {
                    let intent = intent_from_row(row, &cursor.workspace)?;
                    let name = match &intent {
                        Some(intent) => intent.name.clone(),
                        None => row.get(0)?,
                    };
                    Ok((name, intent))
                })?;
            let intents = rows.collect::<rusqlite::Result<Vec<_>>>()?;
            drop(select);
            tx.commit()?;
            intents
        };

        let more = intents.len() > limit;
        let continuation = more.then(|| PinScanCursor {
            workspace: cursor.workspace.clone(),
            revision: cursor.revision,
            after: intents[limit - 1].0.clone(),
        });
        let mut items = Vec::with_capacity(intents.len().min(limit));
        if !intents.is_empty() && catalog_revision(&self.db)? != cursor.revision {
            return Ok(PinPageResult::RestartRequired);
        }
        for intent in intents.into_iter().take(limit).filter_map(|(_, intent)| intent) {
            let mut outcome = reflect(intent.clone(), None).await?;
            let replayed = outcome == SourcePinReflection::ReplayRequired;
            if replayed {
                let Some(replay) = self.replay_pin_at_revision(&intent, Some(cursor.revision))? else {
                    return Ok(PinPageResult::RestartRequired);
                };
                if catalog_revision(&self.db)? != cursor.revision { return Ok(PinPageResult::RestartRequired); }
                outcome = reflect(intent.clone(), Some(replay.source.delta)).await?;
            }
            if catalog_revision(&self.db)? != cursor.revision { return Ok(PinPageResult::RestartRequired); }
            let status = match outcome {
                SourcePinReflection::Durable(receipt) if receipt == intent => PinSyncStatus::Durable { sent_replay: replayed },
                SourcePinReflection::Durable(_) => return Err("source pin receipt does not match the requested intent".into()),
                SourcePinReflection::Superseded => PinSyncStatus::Superseded,
                SourcePinReflection::NameHeldByDirectPin => PinSyncStatus::NameHeldByDirectPin,
                SourcePinReflection::ReplayRequired => PinSyncStatus::ReplayStillRequired,
            };
            items.push(PinPageItem { intent, status });
        }
        // Empty pages have made no daemon requests. Their read transaction
        // supplied the observation cut. Nonempty pages recheck after each
        // effect, before sending the next request.
        Ok(PinPageResult::Page(PinPage { items, continuation }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_page_is_a_bounded_primary_key_window_even_with_legacy_releases() {
        let db = Connection::open_in_memory().unwrap();
        super::super::pins::initialize(&db).unwrap();
        db.execute_batch(
            "ALTER TABLE source_pins ADD COLUMN released_root BLOB;
             INSERT INTO source_pins VALUES ('a', 'app', NULL, 1, NULL),
                 ('b', 'app', NULL, 2, NULL), ('z', 'app', zeroblob(32), 3, NULL);",
        ).unwrap();
        let plan: String = db.query_row(&format!("EXPLAIN QUERY PLAN {PIN_PAGE_SQL}"),
            params!["", 2], |row| row.get(3)).unwrap();
        assert!(plan.contains("USING PRIMARY KEY (name>?)"), "{plan}");
        let mut statement = db.prepare(PIN_PAGE_SQL).unwrap();
        let page: Vec<(String, Option<SourcePinIntent>)> = statement.query_map(params!["", 2], |row| {
            Ok((row.get(0)?, intent_from_row(row, "/workspace")?))
        }).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        assert_eq!(page, vec![("a".into(), None), ("b".into(), None)]);
        let deferred: SourcePinIntent = statement.query_row(params!["b", 1], |row| {
            Ok(intent_from_row(row, "/workspace")?.unwrap())
        }).unwrap();
        assert_eq!(deferred.name, "z");
    }
}
