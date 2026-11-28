//! Local metadata custody for a short publication cut, not builder authority.

use super::{read_selection, AcknowledgedSourceGeneration, Result, SqliteSourceState};
use rusqlite::{Transaction, TransactionBehavior};

/// Holds the SQLite writer reservation for the exact acknowledgement. It
/// does not hold editor files, the local configuration or the artifact store.
pub(crate) struct HeldPublicationSelection<'a> {
    _reservation: Transaction<'a>,
}

impl SqliteSourceState {
    /// Reserve against concurrent local acknowledgement during an already
    /// prepared commit. Drop the returned reservation without a SQLite
    /// write. Never hold it across compiler, linker, or runner execution.
    pub(crate) fn hold_publication_selection(
        &mut self,
        expected: &AcknowledgedSourceGeneration,
    ) -> Result<Option<HeldPublicationSelection<'_>>> {
        let selection = expected.selection();
        if self.project.to_str() != Some(selection.project.as_str()) {
            return Ok(None);
        }
        let reservation = self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !expected.matches(read_selection(&reservation, &selection.target)?) {
            return Ok(None);
        }
        Ok(Some(HeldPublicationSelection { _reservation: reservation }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source_recovery::ClientSourceLimits;
    use rusqlite::{params, Connection, Error, ErrorCode};
    use std::{fs, path::PathBuf, time::{Duration, SystemTime, UNIX_EPOCH}};
    use wosy_protocol::NodeId;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
            let path = std::env::temp_dir().join(format!("wosy-publication-selection-{}-{now}", std::process::id()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn held_exact_acknowledgement_blocks_writer_until_release() {
        let scratch = Scratch::new();
        let limits = ClientSourceLimits { nodes: 8, encoded_bytes: 1024 };
        let mut source = SqliteSourceState::open(&scratch.0, limits).unwrap();
        // Exercise the metadata transaction only, with disposable selected
        // rows. This deliberately does not simulate a validated source graph.
        let old = NodeId([10; 32]);
        let new = NodeId([11; 32]);
        source.db.execute(
            "INSERT INTO source_targets (target, selected, pending, generation, selected_generation)
             VALUES ('app', ?1, NULL, 1, 1)", params![old.0],
        ).unwrap();
        let first = source.capture_acknowledged_source("app").unwrap().unwrap();
        let contender = Connection::open(scratch.0.join(".wosy/client.sqlite")).unwrap();
        contender.busy_timeout(Duration::ZERO).unwrap();
        let replace = |root: NodeId, generation: i64| contender.execute(
            "UPDATE source_targets SET selected = ?1, generation = ?2, selected_generation = ?2
             WHERE target = 'app'", params![root.0, generation],
        );

        let held = source.hold_publication_selection(&first).unwrap().unwrap();
        let error = replace(new, 2).unwrap_err();
        assert!(matches!(error, Error::SqliteFailure(failure, _)
            if failure.code == ErrorCode::DatabaseBusy));
        drop(held);
        assert_eq!(replace(new, 2).unwrap(), 1);
        assert!(source.hold_publication_selection(&first).unwrap().is_none());

        assert_eq!(replace(old, 3).unwrap(), 1);
        assert!(source.hold_publication_selection(&first).unwrap().is_none());
        let current = source.capture_acknowledged_source("app").unwrap().unwrap();
        assert_eq!(first.selection(), current.selection());
        assert_eq!(current.generation(), 3);
        assert!(source.hold_publication_selection(&current).unwrap().is_some());
    }
}
