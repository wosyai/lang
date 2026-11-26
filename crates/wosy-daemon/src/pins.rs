//! Explicit durable pins for admitted source roots, scoped to their workspace.
use super::{
    key, published_source, read_source_config, source_top, retention_plan::SourceRead,
    source_proposal::{matches_source_scope, stage_reconciled_closure},
    source_reconciliation::checked_published_source_header,
    BaseCstAdmissionLimits, PublishReport, Result, SourceStorageWorker, SourceStore, SNAPSHOT_ROOTS,
};
use std::sync::Arc;
use surrealkv::{Durability, LSMIterator, Mode, Transaction};
use wosy_protocol::{Delta, NodeId, SourceConfig, SourcePinIntent, SourcePinReflection, SourcePinRetention};

pub(super) const PINS: u8 = 10;
const OWNER_PREFIX_BYTES: usize = 34;

pub struct SourcePin {
    pub name: String,
    pub root: NodeId,
}

pub(super) fn workspace_of_config(tx: &Transaction, id: NodeId) -> Result<String> {
    workspace_of_source_config(&read_source_config(tx, id)?).map(str::to_owned)
}

pub(super) fn workspace_of_source_config(config: &SourceConfig) -> Result<&str> {
    // Publication matched the entry's authored project to this namespace.
    Ok(&config.namespaces.get(&config.entry.package)
        .ok_or("pin scope is missing its entry namespace")?.project)
}

pub(super) struct StoredPin<'a> {
    root: NodeId,
    owner: &'a [u8],
    reflection: Option<Reflection<'a>>,
}

struct Reflection<'a> {
    generation: i64,
    target: &'a str,
    retention: SourcePinRetention,
}

impl StoredPin<'_> {
    pub fn retained_root(&self) -> Option<NodeId> {
        self.reflection.as_ref().is_none_or(|pin| pin.retention == SourcePinRetention::Retained)
            .then_some(self.root)
    }
}

pub(super) fn decode_pin_value(bytes: &[u8]) -> Result<StoredPin<'_>> {
    let (root, owner) = bytes.split_first_chunk::<32>().ok_or("incomplete source pin")?;
    let root = NodeId(*root);
    // Legacy records append a filesystem path, which cannot begin with NUL.
    let Some(body) = owner.strip_prefix(&[0]) else {
        return Ok(StoredPin { root, owner, reflection: None });
    };
    let (&state, body) = body.split_first().ok_or("incomplete reflected source pin")?;
    let retention = match state {
        1 => SourcePinRetention::Retained,
        2 => SourcePinRetention::Released,
        _ => return Err("unsupported reflected source pin state".into()),
    };
    let (generation, body) = body.split_first_chunk::<8>().ok_or("incomplete reflected source pin generation")?;
    let generation = i64::from_be_bytes(*generation);
    let (length, body) = body.split_first_chunk::<4>().ok_or("incomplete reflected source pin target length")?;
    let (target, owner) = body.split_at_checked(u32::from_be_bytes(*length) as usize)
        .ok_or("incomplete reflected source pin target")?;
    if generation <= 0 { return Err("invalid reflected source pin generation".into()); }
    let target = std::str::from_utf8(target)?;
    Ok(StoredPin { root, owner, reflection: Some(Reflection { generation, target, retention }) })
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
        Ok(Self::new(workspace_of_config(tx, id)?))
    }

    fn new(path: String) -> Self {
        let id = NodeId(blake3::derive_key("wosy source pin workspace v1", path.as_bytes()));
        let mut prefix = [0; OWNER_PREFIX_BYTES];
        prefix[..33].copy_from_slice(&key(PINS, id));
        Self { path, prefix }
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

    fn stored<'a>(&self, bytes: &'a [u8]) -> Result<StoredPin<'a>> {
        let pin = decode_pin_value(bytes)?;
        if pin.owner != self.path.as_bytes() {
            return Err("source pin has the wrong workspace owner".into());
        }
        Ok(pin)
    }

    fn reflected_value(&self, intent: &SourcePinIntent) -> Result<Vec<u8>> {
        let target = intent.source.target.as_bytes();
        let length = u32::try_from(target.len())?.to_be_bytes();
        let state = match intent.retention {
            SourcePinRetention::Retained => 1,
            SourcePinRetention::Released => 2,
        };
        let mut bytes = Vec::with_capacity(46 + target.len() + self.path.len());
        bytes.extend_from_slice(&intent.source.root.0);
        bytes.extend_from_slice(&[0, state]);
        bytes.extend_from_slice(&intent.generation.to_be_bytes());
        bytes.extend_from_slice(&length);
        bytes.extend_from_slice(target);
        bytes.extend_from_slice(self.path.as_bytes());
        Ok(bytes)
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
            let old = tx.get_for_update(&key)?;
            let same = if let Some(value) = old.as_deref() {
                let pin = owner.stored(value)?;
                let same = pin.retained_root() == Some(root);
                if pin.reflection.is_some() && !same {
                    return Err("source pin is managed by an exact client intent".into());
                }
                same
            } else {
                false
            };

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
            if owner.stored(&existing)?.reflection.is_some() {
                return Err("source pin is managed by an exact client intent".into());
            }
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
            if let Some(root) = owner.stored(scan.value()?.as_ref())?.retained_root() {
                pins.push(SourcePin { name, root });
            }
            scan.next()?;
        }
        Ok(pins)
    }
}

enum Plan {
    ReplayRequired,
    Superseded,
    NameHeldByDirectPin,
    Confirmed,
    Commit { transaction: Transaction, _read: Option<SourceRead> },
}

impl SourceStorageWorker {
    /// Reconcile an exact SQLite intent supplied by the authorized local
    /// adapter, not by an unbound socket peer. The root and reflected name
    /// are committed together on cache loss. No SQLite transaction is held
    /// while operating the daemon. This pins only admitted base source.
    pub async fn reflect_source_pin(
        &self, intent: SourcePinIntent, replay: Option<Delta>, limits: BaseCstAdmissionLimits,
    ) -> Result<SourcePinReflection> {
        if intent.generation <= 0 { return Err("source pin intent has no accepted generation".into()); }
        if replay.is_some() && intent.retention == SourcePinRetention::Released {
            return Err("a released source pin cannot carry replay".into());
        }
        let slot = self.try_slot()?;
        self.spawn_reserved_query(slot, None, move |worker| async move {
            let intent = Arc::new(intent);
            let replay = replay.map(Arc::new);
            for conflicts in 0..=1 {
                let requested = Arc::clone(&intent);
                let payload = replay.clone();
                let plan = worker.phase(None, move |store| plan_reflection(store, &requested, payload.as_deref(), limits)).await?;
                match plan {
                    Plan::ReplayRequired => return Ok(SourcePinReflection::ReplayRequired),
                    Plan::Superseded => return Ok(SourcePinReflection::Superseded),
                    Plan::NameHeldByDirectPin => return Ok(SourcePinReflection::NameHeldByDirectPin),
                    Plan::Confirmed => break,
                    Plan::Commit { mut transaction, _read } => match transaction.commit().await {
                        Ok(()) => break,
                        Err(surrealkv::Error::TransactionWriteConflict) if conflicts == 0 => continue,
                        Err(error) => return Err(error.into()),
                    }
                }
            }
            let intent = Arc::try_unwrap(intent).ok().expect("reflection phase completed");
            Ok(SourcePinReflection::Durable(intent))
        }).await?
    }
}

fn plan_reflection(
    store: &SourceStore, intent: &SourcePinIntent, replay: Option<&Delta>, limits: BaseCstAdmissionLimits,
) -> Result<Plan> {
    let source = &intent.source;
    let owner = Owner::new(source.project.clone());
    let key = owner.pin_key(&intent.name)?;
    let retained = intent.retention == SourcePinRetention::Retained;
    let (mut tx, read) = if retained {
        store.begin_pin_update(source.root)?
    } else {
        (store.tree.begin_with_mode(Mode::ReadWrite)?, None)
    };
    let previous = tx.get_for_update(&key)?;
    let mut same = false;
    if let Some(bytes) = previous.as_deref() {
        let pin = owner.stored(bytes)?;
        // A legacy name has independent direct retention and no local
        // generation. Never infer permission to replace or release it.
        let Some(known) = pin.reflection else { return Ok(Plan::NameHeldByDirectPin); };
        if known.generation > intent.generation { return Ok(Plan::Superseded); }
        if known.generation == intent.generation {
            if pin.root != source.root || known.target != source.target {
                return Err("source pin generation has a different root or target".into());
            }
            if known.retention == SourcePinRetention::Released && retained { return Ok(Plan::Superseded); }
            same = known.retention == intent.retention;
        }
    }

    if retained {
        if read.is_some() {
            if !matches_source_scope(&checked_published_source_header(&tx, source.root)?, source) {
                return Err("source pin root does not match its workspace and target".into());
            }
        } else {
            let Some(replay) = replay else { return Ok(Plan::ReplayRequired); };
            stage_reconciled_closure(&mut tx, source, replay, limits, &mut PublishReport::default())?;
            tx.set(&super::key(SNAPSHOT_ROOTS, source.root), &source.root.0)?;
        }
    }
    if same && (!retained || read.is_some()) { return Ok(Plan::Confirmed); }

    // The new closure and pin are in one Immediate transaction. If that
    // closure shares pre-existing dependencies, sync those before commit.
    if retained { store.tree.flush_wal(true)?; }
    tx.set_durability(Durability::Immediate);
    tx.set(&key, &owner.reflected_value(intent)?)?;
    Ok(Plan::Commit { transaction: tx, _read: read })
}
