use super::{IntegerSet, Rc};
use std::hash::{DefaultHasher, Hash, Hasher};
use wosy_syntax::SourceSpan;

type Key = (SourceSpan, Option<usize>);
type Entry = (Key, Rc<IntegerSet>);

/// Sixteen four-bit steps over the hash, followed by an exact key bucket.
/// Call suspension clones only the root handle. Mutation copies only the
/// nodes on the touched key's path, never the previous writes of the frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct StorageDelta(Option<Rc<Node>>);

#[derive(Clone, Debug, PartialEq, Eq)]
enum Node {
    Branch(Vec<(u8, Rc<Node>)>),
    Leaf(Vec<Entry>),
}

fn hash(key: &Key) -> u64 {
    let mut state = DefaultHasher::new();
    key.hash(&mut state);
    state.finish()
}

impl StorageDelta {
    pub(super) fn insert(&mut self, key: Key, value: Rc<IntegerSet>) {
        fn insert(slot: &mut Rc<Node>, level: u8, hash: u64, entry: Entry) {
            #[cfg(test)]
            DELTA_NODE_VISITS.with(|count| count.set(count.get() + 1));
            match Rc::make_mut(slot) {
                Node::Leaf(entries) => {
                    if let Some(known) = entries.iter_mut().find(|(key, _)| key == &entry.0) {
                        known.1 = entry.1;
                    } else {
                        entries.push(entry);
                    }
                }
                Node::Branch(children) => {
                    let digit = (hash & 15) as u8;
                    let child = match children.binary_search_by_key(&digit, |(key, _)| *key) {
                        Ok(index) => &mut children[index].1,
                        Err(index) => {
                            let node = if level == 15 { Node::Leaf(Vec::new()) } else { Node::Branch(Vec::new()) };
                            children.insert(index, (digit, Rc::new(node)));
                            &mut children[index].1
                        }
                    };
                    insert(child, level + 1, hash >> 4, entry);
                }
            }
        }
        let hash = hash(&key);
        insert(self.0.get_or_insert_with(|| Rc::new(Node::Branch(Vec::new()))), 0, hash, (key, value));
    }

    pub(super) fn get(&self, key: &Key) -> Option<&Rc<IntegerSet>> {
        let mut hash = hash(key);
        let mut node = self.0.as_deref()?;
        loop {
            match node {
                Node::Leaf(entries) => return entries.iter().find(|(known, _)| known == key).map(|(_, value)| value),
                Node::Branch(children) => {
                    let digit = (hash & 15) as u8;
                    let index = children.binary_search_by_key(&digit, |(key, _)| *key).ok()?;
                    node = &children[index].1;
                    hash >>= 4;
                }
            }
        }
    }

    pub(super) fn into_entries(self) -> impl Iterator<Item = Entry> {
        let mut pending = Vec::from_iter(self.0);
        let mut bucket = Vec::new().into_iter();
        std::iter::from_fn(move || loop {
            if let Some(entry) = bucket.next() { return Some(entry); }
            match Rc::unwrap_or_clone(pending.pop()?) {
                Node::Leaf(entries) => bucket = entries.into_iter(),
                Node::Branch(children) => pending.extend(children.into_iter().rev().map(|(_, node)| node)),
            }
        })
    }
}

#[cfg(test)]
thread_local! { static DELTA_NODE_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

#[cfg(test)]
#[test]
fn nested_suspended_storage_versions_only_copy_a_fixed_length_key_path() {
    use num_bigint::BigInt;
    use wosy_syntax::{ByteSpan, SourceIdentity};
    let declaration = SourceSpan::new(
        SourceIdentity { project: "local".into(), package: "app".into(), path: "src/main.w".into(), revision: "nested-storage".into() },
        ByteSpan::new(1, 2),
    );
    for depth in [32usize, 256, 1024] {
        let mut parents = Vec::new();
        let mut latest = StorageDelta::default();
        DELTA_NODE_VISITS.with(|count| count.set(0));
        for cell in 0..depth {
            parents.push(latest.clone());
            latest.insert((declaration.clone(), Some(cell)), Rc::new(IntegerSet::singleton(&BigInt::from(cell))));
        }
        let visits = DELTA_NODE_VISITS.with(|count| count.replace(0));
        assert_eq!(visits, 17 * depth);
        for (cell, parent) in parents.into_iter().enumerate() {
            assert!(parent.get(&(declaration.clone(), Some(cell))).is_none());
            if cell > 0 {
                assert_eq!(parent.get(&(declaration.clone(), Some(cell - 1))).unwrap().singleton_value(), Some(&BigInt::from(cell - 1)));
            }
        }
        assert_eq!(latest.into_entries().count(), depth);
        eprintln!("nested storage frames={depth} delta path visits={visits}");
    }
}
