//! Process-local evidence for the canonical authored grammar check only.
use std::{collections::{HashSet, VecDeque}, sync::Mutex};
use wosy_protocol::{FileEntry, NodeId};

const RECEIPTS: usize = 2048;

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
struct GrammarKey {
    // The verified File record binds owner, native FileKey, revision, and
    // exact authored, executable and annotation nodes.
    file: NodeId,
    parser: NodeId,
}

impl GrammarKey {
    fn for_file(file: &FileEntry) -> Self {
        Self { file: file.file, parser: file.metadata.policy }
    }
}

#[derive(Default)]
pub(super) struct VerifiedGrammarBatch(Vec<GrammarKey>);

impl VerifiedGrammarBatch {
    /// Only after the real canonical grammar check accepted this file.
    pub fn verified(&mut self, file: &FileEntry) {
        if self.0.len() < RECEIPTS { self.0.push(GrammarKey::for_file(file)); }
    }
}

#[derive(Default)]
pub(super) struct GrammarReceipts(Mutex<Receipts>);

#[derive(Default)]
struct Receipts {
    keys: HashSet<GrammarKey>,
    fifo: VecDeque<GrammarKey>,
}

impl Receipts {
    fn remember(&mut self, key: GrammarKey) {
        if self.keys.contains(&key) { return; }
        if self.keys.len() == RECEIPTS {
            let oldest = self.fifo.pop_front().expect("full receipt queue");
            self.keys.remove(&oldest);
        }
        self.keys.insert(key);
        self.fifo.push_back(key);
    }
}

impl GrammarReceipts {
    pub fn contains(&self, file: &FileEntry) -> bool {
        self.0.lock().expect("grammar receipt memo").keys.contains(&GrammarKey::for_file(file))
    }

    /// One open SourceStore and running parser implementation. Nothing is
    /// serialized, carried over to another process, or inferred from a
    /// generic published/legacy marker. No validation runs under this lock.
    pub fn accept(&self, batch: VerifiedGrammarBatch) {
        if batch.0.is_empty() { return; }
        let mut receipts = self.0.lock().expect("grammar receipt memo");
        for key in batch.0 { receipts.remember(key); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(file: usize, parser: u8) -> GrammarKey {
        let mut id = [0; 32];
        id[..std::mem::size_of::<usize>()].copy_from_slice(&file.to_be_bytes());
        GrammarKey { file: NodeId(id), parser: NodeId([parser; 32]) }
    }

    #[test]
    fn bounded_fifo_keeps_grammar_policy_exact() {
        let mut receipts = Receipts::default();
        for file in 0..RECEIPTS {
            receipts.remember(key(file, 1));
        }
        assert_eq!(receipts.keys.len(), RECEIPTS);
        assert_eq!(receipts.fifo.len(), RECEIPTS);
        assert!(!receipts.keys.contains(&key(0, 2)));
        receipts.remember(key(0, 1));
        receipts.remember(key(0, 2));
        assert_eq!(receipts.keys.len(), RECEIPTS);
        assert_eq!(receipts.fifo.len(), RECEIPTS);
        assert!(!receipts.keys.contains(&key(0, 1)));
        assert!(receipts.keys.contains(&key(0, 2)));
        assert!(receipts.keys.contains(&key(1, 1)));
    }
}
