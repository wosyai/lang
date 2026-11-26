//! Process-local evidence for the canonical authored grammar check only.
use std::{collections::{HashMap, VecDeque}, sync::{Mutex, MutexGuard}};
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
pub(super) struct VerifiedGrammarBatch(Vec<(GrammarKey, bool)>);

impl VerifiedGrammarBatch {
    pub fn is_empty(&self) -> bool { self.0.is_empty() }

    /// Only after the real canonical grammar check accepted this file
    /// and observed its authored syntax in the same equality walk.
    pub fn verified(&mut self, file: &FileEntry, authored_comptime: bool) {
        if self.0.len() < RECEIPTS { self.0.push((GrammarKey::for_file(file), authored_comptime)); }
    }
}

#[derive(Default)]
pub(super) struct GrammarReceipts(Mutex<Receipts>);

pub(super) struct GrammarReceiptAdmission<'a>(MutexGuard<'a, Receipts>);

#[derive(Default)]
struct Receipts {
    keys: HashMap<GrammarKey, bool>,
    fifo: VecDeque<GrammarKey>,
}

impl Receipts {
    fn remember(&mut self, key: GrammarKey, authored_comptime: bool) {
        if self.keys.contains_key(&key) { return; }
        if self.keys.len() == RECEIPTS {
            let oldest = self.fifo.pop_front().expect("full receipt queue");
            self.keys.remove(&oldest);
        }
        self.keys.insert(key, authored_comptime);
        self.fifo.push_back(key);
    }
}

impl GrammarReceipts {
    pub fn contains(&self, file: &FileEntry) -> bool {
        self.authored_comptime(file).is_some()
    }

    // None is missing authority, never evidence of an empty source.
    pub fn authored_comptime(&self, file: &FileEntry) -> Option<bool> {
        self.0.lock().expect("grammar receipt memo").keys.get(&GrammarKey::for_file(file)).copied()
    }

    /// One open SourceStore and running parser implementation. Nothing is
    /// serialized, carried over to another process, or inferred from a
    /// generic published/legacy marker. No validation runs under this lock.
    pub fn accept(&self, batch: VerifiedGrammarBatch) {
        if !batch.is_empty() { self.admission().accept(batch); }
    }

    pub fn admission(&self) -> GrammarReceiptAdmission<'_> {
        GrammarReceiptAdmission(self.0.lock().expect("grammar receipt memo"))
    }
}

impl GrammarReceiptAdmission<'_> {
    pub fn accept(&mut self, batch: VerifiedGrammarBatch) {
        for (key, authored_comptime) in batch.0 { self.0.remember(key, authored_comptime); }
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
            receipts.remember(key(file, 1), file % 2 == 0);
        }
        assert_eq!(receipts.keys.len(), RECEIPTS);
        assert_eq!(receipts.fifo.len(), RECEIPTS);
        assert!(!receipts.keys.contains_key(&key(0, 2)));
        receipts.remember(key(0, 1), true);
        receipts.remember(key(0, 2), false);
        assert_eq!(receipts.keys.len(), RECEIPTS);
        assert_eq!(receipts.fifo.len(), RECEIPTS);
        assert!(!receipts.keys.contains_key(&key(0, 1)));
        assert_eq!(receipts.keys.get(&key(0, 2)), Some(&false));
        assert_eq!(receipts.keys.get(&key(1, 1)), Some(&false));
        assert_eq!(receipts.keys.get(&key(2, 1)), Some(&true));
    }
}
