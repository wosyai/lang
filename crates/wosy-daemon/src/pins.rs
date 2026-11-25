//! Explicit durable pins for admitted source roots, scoped to their workspace.
use super::{key, published_source, read_node, source_top, Result, SourceStore, SNAPSHOT_ROOTS};
use surrealkv::{Durability, LSMIterator, Mode, Transaction};
use wosy_protocol::{NodeId, SourceConfig};

pub(super) const PINS: u8 = 10;
const OWNER_PREFIX_BYTES: usize = 34;

pub struct SourcePin {
    pub name: String,
    pub root: NodeId,
}

pub(super) fn workspace_of_config(tx: &Transaction, id: NodeId) -> Result<String> {
    let mut config: SourceConfig = serde_json::from_slice(&read_node(tx, id)?.data)?;
    // Publication matched the entry's authored project to this namespace.
    Ok(config.namespaces.remove(&config.entry.package)
        .ok_or("pin scope is missing its entry namespace")?.project)
}

pub(super) fn decode_pin_value(bytes: &[u8]) -> Result<(NodeId, &[u8])> {
    let (root, owner) = bytes.split_first_chunk::<32>().ok_or("incomplete source pin")?;
    Ok((NodeId(*root), owner))
}

struct Owner {
    path: String,
    prefix: [u8; OWNER_PREFIX_BYTES],
}

impl Owner {
    fn of_source(tx: &Transaction, root: NodeId) -> Result<Self> {
        let source = published_source(tx, root)?.ok_or("pin scope needs a published source root")?;
        Self::from_config(tx, source.children[0])
    }

    fn for_update(tx: &mut Transaction, root: NodeId) -> Result<Self> {
        // Ordinary get() does not register an optimistic read conflict in
        // SurrealKV. Keep root removal visible to a future retention writer.
        tx.get_for_update(&key(SNAPSHOT_ROOTS, root))?.ok_or("pin scope needs a published source root")?;
        let source = source_top(tx, root)?.ok_or("unsupported source root for pinning")?;
        Self::from_config(tx, source.children[0])
    }

    fn from_config(tx: &Transaction, id: NodeId) -> Result<Self> {
        let path = workspace_of_config(tx, id)?;
        let id = NodeId(blake3::derive_key("wosy source pin workspace v1", path.as_bytes()));
        let mut prefix = [0; OWNER_PREFIX_BYTES];
        prefix[..33].copy_from_slice(&key(PINS, id));
        Ok(Self { path, prefix })
    }

    fn pin_key(&self, name: &str) -> Result<Vec<u8>> {
        if name.is_empty() { return Err("pin name is empty".into()); }
        let mut key = Vec::with_capacity(OWNER_PREFIX_BYTES + name.len());
        key.extend_from_slice(&self.prefix);
        key.extend_from_slice(name.as_bytes());
        Ok(key)
    }

    fn upper_bound(&self) -> [u8; OWNER_PREFIX_BYTES] {
        let mut upper = self.prefix;
        upper[OWNER_PREFIX_BYTES - 1] = 1;
        upper
    }

    fn value(&self, root: NodeId) -> Vec<u8> {
        let mut value = Vec::with_capacity(32 + self.path.len());
        value.extend_from_slice(&root.0);
        value.extend_from_slice(self.path.as_bytes());
        value
    }

    fn pinned_root(&self, bytes: &[u8]) -> Result<NodeId> {
        let (root, owner) = decode_pin_value(bytes)?;
        if owner != self.path.as_bytes() {
            return Err("source pin has the wrong workspace owner".into());
        }
        Ok(root)
    }
}

impl SourceStore {
    /// Pin only a published source root in its own workspace. This is a
    /// storage operation, not a transport authorization boundary.
    pub async fn pin_source(&self, root: NodeId, name: &str) -> Result<SourcePin> {
        loop {
            let mut tx = self.tree.begin()?;
            let owner = Owner::for_update(&mut tx, root)?;
            let key = owner.pin_key(name)?;
            let pin = SourcePin { name: name.to_owned(), root };
            let same = tx.get_for_update(&key)?.as_deref().map(|bytes| owner.pinned_root(bytes))
                .transpose()? == Some(root);

            // Admit the already committed source closure first. The pinned
            // source graph is entirely inline with the value log disabled.
            // Optional AST CAS files are not dependencies of this source pin.
            self.tree.flush_wal(true)?;
            // Reconfirm observed pins too: visibility alone does not certify
            // the WAL has been synced to this call's point of observation.
            tx.set_durability(Durability::Immediate);
            if !same { tx.set(&key, owner.value(root).as_slice())?; }
            match tx.commit().await {
                Ok(()) => return Ok(pin),
                Err(surrealkv::Error::TransactionWriteConflict) => continue,
                Err(error) => return Err(error.into()),
            }
        }
    }

    /// Release one named pin; this does not delete the underlying snapshot.
    pub async fn unpin_source(&self, scope: NodeId, name: &str) -> Result<bool> {
        loop {
            let mut tx = self.tree.begin()?;
            let owner = Owner::for_update(&mut tx, scope)?;
            let key = owner.pin_key(name)?;
            let Some(existing) = tx.get(&key)? else { return Ok(false); };
            owner.pinned_root(&existing)?;
            tx.set_durability(Durability::Immediate);
            tx.delete(&key)?;
            match tx.commit().await {
                Ok(()) => return Ok(true),
                Err(surrealkv::Error::TransactionWriteConflict) => continue,
                Err(error) => return Err(error.into()),
            }
        }
    }

    pub fn source_pins(&self, scope: NodeId) -> Result<Vec<SourcePin>> {
        let tx = self.tree.begin_with_mode(Mode::ReadOnly)?;
        let owner = Owner::of_source(&tx, scope)?;
        let mut scan = tx.range(owner.prefix.as_slice(), owner.upper_bound().as_slice())?;
        let mut pins = Vec::new();
        scan.seek_first()?;
        while scan.valid() {
            let name = std::str::from_utf8(&scan.key().user_key()[OWNER_PREFIX_BYTES..])?.to_owned();
            let root = owner.pinned_root(scan.value()?.as_ref())?;
            pins.push(SourcePin { name, root });
            scan.next()?;
        }
        Ok(pins)
    }
}
