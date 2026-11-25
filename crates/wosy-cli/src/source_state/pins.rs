//! Explicit local retention of acknowledged source-preparation roots. The
//! caller reflects a local pin to the daemon separately; there is no atomic
//! commit across the project client and a daemon store.
use super::{read_selection, Result, SourceReplay, SqliteSourceState};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use wosy_protocol::NodeId;

#[derive(Debug, Eq, PartialEq)]
pub struct LocalSourcePin {
    pub name: String,
    pub target: String,
    pub root: NodeId,
    pub generation: i64,
}

pub struct PinnedSourceReplay {
    pub pin: LocalSourcePin,
    pub replay: SourceReplay,
}

pub(super) fn initialize(sql: &Connection) -> Result<()> {
    sql.execute_batch(
        "CREATE TABLE source_pins (
            name TEXT PRIMARY KEY,
            target TEXT NOT NULL,
            root BLOB,
            generation INTEGER NOT NULL CHECK (generation > 0)
         ) STRICT, WITHOUT ROWID;
         PRAGMA main.user_version = 3;",
    )?;
    Ok(())
}

impl SqliteSourceState {
    /// Name and retain the already acknowledged source selection for this
    /// target. This is not a pin on a compiled artifact or final SnapshotRoot.
    pub fn pin_selected_source(&mut self, target: &str, name: &str) -> Result<LocalSourcePin> {
        if name.is_empty() { return Err("pin name is empty".into()); }
        let root = self.selection(target)?.selected.ok_or("cannot pin a target without an acknowledged source root")?;
        // A pin must be recoverable. Do not turn a cheap parser observation
        // or an unacknowledged proposal into retained source authority.
        self.load_root(target, root, None)?;
        let tx = self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if read_selection(&tx, target)?.selected != Some(root) {
            return Err("the selected source changed while preparing a local pin".into());
        }
        let generation: Option<i64> = tx.query_row(
            "SELECT generation FROM source_pins WHERE name = ?1", [name], |row| row.get(0),
        ).optional()?;
        let generation = generation.unwrap_or_default().checked_add(1).ok_or("local source pin generation exhausted")?;
        tx.execute(
            "INSERT INTO source_pins (name, target, root, generation) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(name) DO UPDATE
             SET target = excluded.target, root = excluded.root, generation = excluded.generation",
            params![name, target, root.0, generation],
        )?;
        tx.commit()?;
        Ok(LocalSourcePin { name: name.to_owned(), target: target.to_owned(), root, generation })
    }

    pub fn local_source_pins(&self) -> Result<Vec<LocalSourcePin>> {
        let mut select = self.db.prepare("SELECT name, target, root, generation FROM source_pins
            WHERE root IS NOT NULL ORDER BY name")?;
        let rows = select.query_map([], read_pin)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Rebuild one named pin from immutable local bytes, using that pin's own
    /// target, even if the edit selected in this target or its manifest changed.
    pub fn replay_local_source_pin(&self, name: &str) -> Result<Option<PinnedSourceReplay>> {
        let pin = self.db.query_row(
            "SELECT name, target, root, generation FROM source_pins WHERE name = ?1 AND root IS NOT NULL",
            [name], read_pin,
        ).optional()?;
        pin.map(|pin| {
            let replay = self.load_root(&pin.target, pin.root, None)?;
            Ok(PinnedSourceReplay { pin, replay })
        }).transpose()
    }

    /// Release only this exact local pin generation. Keep its name generation
    /// so an earlier release cannot clear a later pin of the same source.
    /// This neither removes source bytes nor mutates the daemon's own pins.
    pub fn unpin_local_source(&self, pin: &LocalSourcePin) -> Result<bool> {
        Ok(self.db.execute(
            "UPDATE source_pins SET root = NULL
             WHERE name = ?1 AND target = ?2 AND root = ?3 AND generation = ?4",
            params![pin.name, pin.target, pin.root.0, pin.generation],
        )? == 1)
    }
}

fn read_pin(row: &rusqlite::Row<'_>) -> rusqlite::Result<LocalSourcePin> {
    Ok(LocalSourcePin {
        name: row.get(0)?,
        target: row.get(1)?,
        root: NodeId(row.get(2)?),
        generation: row.get(3)?,
    })
}
