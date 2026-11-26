//! Project-local source selection and recovery. Compiler queries and the
//! public current.json view are not stored here.
use crate::source_preparation::{prepare_with_files, Files, PreparedSources};
use crate::source_recovery::{
    sparse_delta, trace_reachable, ClientSourceLimits, ClientSourceUsage, Closure, RetainedSources,
};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, Statement, TransactionBehavior};
use std::path::{Path, PathBuf};
use wosy_protocol::{source_preparation_parts, Delta, FileEntry, FileKey, Kind, Node, NodeId, SourceConfig};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

mod delta;
mod pins;
pub use pins::{LocalSourcePin, PinnedSourceReplay};

pub struct SqliteSourceState {
    project: PathBuf,
    db: Connection,
    limits: ClientSourceLimits,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SourceSelection {
    pub selected: Option<NodeId>,
    pub pending: Option<SourceAttempt>,
    generation: i64,
}

/// A local upload attempt is distinct from its immutable source root. A
/// cancelled attempt cannot confirm a later attempt with identical bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceAttempt {
    root: NodeId,
    generation: i64,
}

impl SourceAttempt {
    pub fn new(root: NodeId, generation: i64) -> Self {
        Self { root, generation }
    }

    pub fn root(self) -> NodeId { self.root }
    pub fn generation(self) -> i64 { self.generation }
}

pub struct SourceStage {
    pub root: NodeId,
    pub parsed: usize,
    pub reused: usize,
    pub new_nodes: usize,
    pub new_bytes: usize,
    pub storage_reads: usize,
    pub pending: Option<SourceAttempt>,
}

#[derive(Clone, Copy)]
pub enum ReplaySource {
    Selected,
    Pending,
}

pub struct SourceReplay {
    pub source: PreparedSources,
    pub usage: ClientSourceUsage,
    pub attempt: Option<SourceAttempt>,
}

struct Observation {
    source: PreparedSources,
    usage_to_record: Option<ClientSourceUsage>,
    storage_reads: usize,
}

impl SqliteSourceState {
    /// Explicit opt-in, separate from ordinary Parse, Build, Check and Run.
    /// Uses ordinary project-owned files on a local Unix filesystem supporting
    /// SQLite rollback journals. It does not promise power-loss recovery on
    /// arbitrary filesystem or storage configurations.
    pub fn open(project: &Path, limits: ClientSourceLimits) -> Result<Self> {
        let project = project.canonicalize()?;
        let working = project.join(".wosy");
        let cst = working.join("cst");
        let database = working.join("client.sqlite");
        let content = cst.join("nodes.sqlite");
        let location = SqliteLocation::prepare(&working, &cst, [&database, &content])?;
        let mut db = Connection::open_with_flags(&database, OpenFlags::default() | OpenFlags::SQLITE_OPEN_NOFOLLOW)?;
        db.execute("ATTACH DATABASE ?1 AS cst", [content.to_str().ok_or("CST path is not UTF-8")?])?;
        location.check_database(&database)?;
        location.check_database(&content)?;
        // Attached SQLite commits are atomic across both files with a
        // disk-backed main database and rollback journals, not with WAL.
        for sql in ["PRAGMA main.journal_mode", "PRAGMA cst.journal_mode"] {
            let mode: String = db.query_row(sql, [], |row| row.get(0))?;
            if mode != "delete" { return Err("local source state requires SQLite rollback journals".into()); }
        }
        db.execute_batch("PRAGMA main.synchronous = EXTRA; PRAGMA cst.synchronous = EXTRA;")?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: i64 = tx.query_row("PRAGMA main.user_version", [], |row| row.get(0))?;
        let content_version: i64 = tx.query_row("PRAGMA cst.user_version", [], |row| row.get(0))?;
        match (version, content_version) {
            (0, 0) => tx.execute_batch(
                "CREATE TABLE source_targets (
                    target TEXT PRIMARY KEY,
                    selected BLOB,
                    pending BLOB,
                    generation INTEGER NOT NULL CHECK (generation > 0)
                 ) STRICT, WITHOUT ROWID;
                 CREATE TABLE cst.nodes (
                    digest BLOB PRIMARY KEY,
                    payload BLOB NOT NULL
                 ) STRICT, WITHOUT ROWID;
                 PRAGMA main.user_version = 1;
                 PRAGMA cst.user_version = 1;",
            )?,
            (1 | 2 | 3, 1) => {}
            _ => return Err("incompatible local source state schema".into()),
        }
        if version < 2 {
            tx.execute_batch(
                "CREATE TABLE source_root_usage (root BLOB PRIMARY KEY, receipt BLOB NOT NULL)
                 STRICT, WITHOUT ROWID;
                 PRAGMA main.user_version = 2;",
            )?;
        }
        if version < 3 {
            pins::initialize(&tx)?;
        }
        tx.commit()?;
        Ok(Self { project, db, limits })
    }

    pub fn selection(&self, target: &str) -> Result<SourceSelection> {
        read_selection(&self.db, target)
    }

    /// Record complete canonical bytes on first observation, and only new
    /// bytes on a later edit. No SQLite transaction is held while parsing.
    /// Neither an existing confirmed root nor an earlier pending upload can
    /// be silently replaced by a concurrent observer.
    pub fn stage(&mut self, target: &str) -> Result<SourceStage> {
        let before = self.selection(target)?;
        if before.pending.is_some() { return Err("an exact source upload is already pending".into()); }
        if let Some(root) = before.selected {
            let observation = self.observe_again(target, root)?;
            self.record(target, before, observation)
        } else {
            let initial = RetainedSources::observe(&self.project, Some(target), self.limits)?;
            let usage_to_record = Some(initial.usage());
            self.record(target, before, Observation {
                source: initial.into_prepared(), usage_to_record, storage_reads: 0,
            })
        }
    }

    fn record(&mut self, target: &str, prior: SourceSelection, observation: Observation) -> Result<SourceStage> {
        let Observation {
            source: PreparedSources { root, delta, parsed, reused }, usage_to_record, storage_reads,
        } = observation;
        let pending = if prior.selected != Some(root) {
            let generation = prior.generation.checked_add(1).ok_or("local source generation exhausted")?;
            Some(SourceAttempt::new(root, generation))
        } else { None };
        let behavior = if pending.is_some() || usage_to_record.is_some() {
            TransactionBehavior::Immediate
        } else { TransactionBehavior::Deferred };
        let tx = self.db.transaction_with_behavior(behavior)?;
        if read_selection(&tx, target)? != prior {
            return Err("local source selection advanced during observation".into());
        }
        let (mut new_nodes, mut new_bytes) = (0, 0);
        if let Some(attempt) = pending {
            let mut insert = tx.prepare(
                "INSERT INTO cst.nodes (digest, payload) VALUES (?1, ?2) ON CONFLICT(digest) DO NOTHING",
            )?;
            let mut equal = tx.prepare("SELECT payload = ?2 FROM cst.nodes WHERE digest = ?1")?;
            for (id, bytes) in delta.nodes {
                if insert_exact(&mut insert, &mut equal, id, &bytes, "stored CST bytes differ from admitted source")? {
                    new_nodes += 1;
                    new_bytes += bytes.len();
                }
            }
            tx.execute(
                "INSERT INTO source_targets (target, pending, generation) VALUES (?1, ?2, ?3)
                 ON CONFLICT(target) DO UPDATE SET pending = excluded.pending, generation = excluded.generation",
                params![target, attempt.root.0, attempt.generation],
            )?;
        }
        if let Some(usage) = usage_to_record {
            let mut insert = tx.prepare(
                "INSERT INTO source_root_usage (root, receipt) VALUES (?1, ?2)
                 ON CONFLICT(root) DO NOTHING",
            )?;
            let mut equal = tx.prepare("SELECT receipt = ?2 FROM source_root_usage WHERE root = ?1")?;
            insert_exact(&mut insert, &mut equal, root, &usage_receipt(root, usage)?,
                "stored source observation receipt differs from admitted source")?;
        }
        tx.commit()?;
        Ok(SourceStage { root, parsed, reused, new_nodes, new_bytes, storage_reads, pending })
    }

    /// Bind only SQLite's acknowledged target selection to this canonical
    /// workspace. Pending source attempts cannot feed compiler requests.
    pub fn acknowledged_build_source(&self, target: &str) -> Result<Option<wosy_protocol::BuildSourceSelection>> {
        let Some(root) = self.selection(target)?.selected else { return Ok(None); };
        let project = self.project.to_str().ok_or("workspace path is not UTF-8")?;
        Ok(Some(wosy_protocol::BuildSourceSelection { root, project: project.into(), target: target.into() }))
    }

    /// A complete resubmission for a reopened client and an empty or reopened
    /// daemon cache. This does not read the live workspace or invoke a parser.
    pub fn replay(&self, target: &str, which: ReplaySource) -> Result<Option<SourceReplay>> {
        let state = self.selection(target)?;
        let (root, attempt) = match which {
            ReplaySource::Selected => (state.selected, None),
            ReplaySource::Pending => (state.pending.map(SourceAttempt::root), state.pending),
        };
        root.map(|root| self.load_root(target, root, attempt)).transpose()
    }

    /// The caller passes the exact root acknowledged by the source store, not
    /// a derived Build/Check result, cache hit, or artifact.
    pub fn acknowledge(&self, target: &str, attempt: SourceAttempt) -> Result<bool> {
        Ok(self.db.execute(
            "UPDATE source_targets SET selected = ?2, pending = NULL
             WHERE target = ?1 AND pending = ?2 AND generation = ?3",
            params![target, attempt.root.0, attempt.generation],
        )? == 1)
    }

    /// Discard only the specified failed or canceled proposal, retaining the
    /// previous selected root and every already-authored immutable node.
    pub fn abandon(&self, target: &str, attempt: SourceAttempt) -> Result<bool> {
        Ok(self.db.execute(
            "UPDATE source_targets SET pending = NULL WHERE target = ?1 AND pending = ?2 AND generation = ?3",
            params![target, attempt.root.0, attempt.generation],
        )? == 1)
    }

    fn load_root(&self, target: &str, root: NodeId, attempt: Option<SourceAttempt>) -> Result<SourceReplay> {
        let mut read = StoredNodes::new(&self.db)?;
        Self::header(&self.project, &mut read, target, root)?;
        let (delta, usage) = read.closure(root, self.limits)?;
        Ok(SourceReplay { source: PreparedSources { root, delta, parsed: 0, reused: 0 }, usage, attempt })
    }

    fn header(project: &Path, read: &mut StoredNodes<'_>, target: &str, root: NodeId) -> Result<(SourceConfig, Vec<NodeId>)> {
        let top = read.record(root)?;
        let parts = source_preparation_parts(&top).ok_or("unsupported persisted source schema")?;
        let config = read.record(parts.config)?;
        if config.kind != Kind::Config { return Err("expected persisted source configuration".into()); }
        let config: SourceConfig = serde_json::from_slice(&config.data)?;
        if config.target != target || top.data != config.schema()
            || config.parser.schema.0 != *wosy_syntax::base_cst_schema()
            || config.namespaces.get(&config.entry.package).is_none_or(|ns| {
                ns.git.is_some() || Path::new(&ns.project) != project
            })
        {
            return Err("persisted source does not match this project, target, or parser".into());
        }
        Ok((config, parts.files.to_vec()))
    }

    fn observe_again(&self, target: &str, root: NodeId) -> Result<Observation> {
        let mut read = StoredNodes::new(&self.db)?;
        let (config, entries) = Self::header(&self.project, &mut read, target, root)?;
        let budget = load_usage(&self.db, root)?;
        if let Some(usage) = budget { usage.admit(self.limits)?; }
        let files = read.file_index(&config, entries)?;
        let (mut source, _) = prepare_with_files(&self.project, Some(target), Some(&files))?;
        // An unchanged observation only compares the selected root. An
        // attempt to use or publish canonical bytes must still visit the
        // complete hash-checked closure, regardless of this budget receipt.
        let usage_to_record = if source.root != root {
            let (base, _) = read.closure(root, self.limits)?;
            Some(sparse_delta(&mut source, &base, self.limits)?.usage())
        } else if budget.is_none() {
            // A v1 database has no recorded usage. Validate once before
            // storing a receipt rather than assuming the earlier limits.
            Some(read.closure(root, self.limits)?.1)
        } else { None };
        Ok(Observation { source, usage_to_record, storage_reads: read.count })
    }
}

/// A deduplication hit must be byte-identical. It can belong to a historical
/// root outside the currently validated base closure.
fn insert_exact(
    insert: &mut Statement<'_>, equal: &mut Statement<'_>, id: NodeId, bytes: &[u8], mismatch: &'static str,
) -> Result<bool> {
    if insert.execute(params![id.0, bytes])? == 1 { return Ok(true); }
    if !equal.query_row(params![id.0, bytes], |row| row.get::<_, bool>(0))? {
        return Err(mismatch.into());
    }
    Ok(false)
}

struct StoredNodes<'a> {
    query: Statement<'a>,
    checked: Delta,
    count: usize,
}

impl<'a> StoredNodes<'a> {
    fn new(db: &'a Connection) -> Result<Self> {
        Ok(Self { query: db.prepare("SELECT payload FROM cst.nodes WHERE digest = ?1")?, checked: Delta::default(), count: 0 })
    }

    fn fetch(&mut self, id: NodeId) -> Result<Vec<u8>> {
        self.count += 1;
        let bytes: Vec<u8> = self.query.query_row([id.0], |row| row.get(0)).optional()?
            .ok_or("persisted source generation is not closed")?;
        if Node::id(&bytes) != id { return Err("persisted source node hash mismatch".into()); }
        Ok(bytes)
    }

    fn record(&mut self, id: NodeId) -> Result<Node> {
        let decode = |bytes: &[u8]| Node::decode(bytes).ok_or("invalid persisted source record");
        if let Some(bytes) = self.checked.nodes.get(&id) {
            return Ok(decode(bytes)?);
        }
        let bytes = self.fetch(id)?;
        let record = decode(&bytes)?;
        self.checked.nodes.insert(id, bytes);
        Ok(record)
    }

    fn file_index(&mut self, config: &SourceConfig, entries: Vec<NodeId>) -> Result<Files> {
        let parser = config.parser.id();
        let mut files = Files::new();
        for id in entries {
            let file = FileEntry::from_node(id, self.record(id)?)?;
            let key = FileKey::from(&file.metadata.source);
            if file.metadata.policy != parser
                || config.namespaces.get(&key.package).is_none_or(|namespace| {
                    namespace.project != file.metadata.source.project
                })
                || files.insert(key, file).is_some()
            {
                return Err("invalid persisted source file index".into());
            }
        }
        if !files.contains_key(&config.entry) { return Err("persisted entry file is missing".into()); }
        Ok(files)
    }

    fn closure(&mut self, root: NodeId, limits: ClientSourceLimits) -> Result<(Delta, ClientSourceUsage)> {
        let mut nodes = Delta::default();
        let closure = self.trace(root, limits, |id, bytes| { nodes.nodes.insert(id, bytes); })?;
        Ok((nodes, closure.usage()))
    }

    fn trace(
        &mut self, root: NodeId, limits: ClientSourceLimits, keep: impl FnMut(NodeId, Vec<u8>),
    ) -> Result<Closure> {
        trace_reachable(root, limits, |id| match self.checked.nodes.remove(&id) {
            Some(bytes) => Ok(bytes),
            None => self.fetch(id),
        }, keep)
    }
}

fn usage_digest(root: NodeId, counts: [u8; 16]) -> [u8; 32] {
    let mut hash = blake3::Hasher::new_derive_key("wosy selected source observation usage v1");
    hash.update(&root.0);
    hash.update(&counts);
    *hash.finalize().as_bytes()
}

fn usage_receipt(root: NodeId, usage: ClientSourceUsage) -> Result<[u8; 48]> {
    let mut counts = [0; 16];
    counts[..8].copy_from_slice(&u64::try_from(usage.nodes)?.to_be_bytes());
    counts[8..].copy_from_slice(&u64::try_from(usage.encoded_bytes)?.to_be_bytes());
    let mut receipt = [0; 48];
    receipt[..16].copy_from_slice(&counts);
    receipt[16..].copy_from_slice(&usage_digest(root, counts));
    Ok(receipt)
}

fn load_usage(db: &Connection, root: NodeId) -> Result<Option<ClientSourceUsage>> {
    let receipt: Option<[u8; 48]> = db.query_row(
        "SELECT receipt FROM source_root_usage WHERE root = ?1", [root.0], |row| row.get(0),
    ).optional()?;
    let Some(receipt) = receipt else { return Ok(None); };
    let counts: [u8; 16] = receipt[..16].try_into()?;
    if usage_digest(root, counts) != receipt[16..] { return Err("invalid source observation usage receipt".into()); }
    let nodes = usize::try_from(u64::from_be_bytes(counts[..8].try_into()?))?;
    let encoded_bytes = usize::try_from(u64::from_be_bytes(counts[8..].try_into()?))?;
    if nodes == 0 || encoded_bytes == 0 { return Err("invalid source observation usage receipt".into()); }
    Ok(Some(ClientSourceUsage { nodes, encoded_bytes }))
}

fn read_selection(sql: &Connection, target: &str) -> Result<SourceSelection> {
    Ok(sql.query_row(
        "SELECT selected, pending, generation FROM source_targets WHERE target = ?1", [target], |row| {
            let generation = row.get(2)?;
            Ok(SourceSelection {
                selected: row.get::<_, Option<[u8; 32]>>(0)?.map(NodeId),
                pending: row.get::<_, Option<[u8; 32]>>(1)?
                    .map(|root| SourceAttempt::new(NodeId(root), generation)),
                generation,
            })
        },
    ).optional()?.unwrap_or_default())
}

struct SqliteLocation {
    #[cfg(unix)]
    device: u64,
}

#[cfg(unix)]
impl SqliteLocation {
    fn prepare(working: &Path, cst: &Path, databases: [&Path; 2]) -> Result<Self> {
        use std::os::unix::fs::MetadataExt as _;

        let working = plain_directory(working)?;
        let cst = plain_directory(cst)?;
        let device = working.dev();
        if cst.dev() != device { return Err("local source state and CST must share a filesystem".into()); }
        let location = Self { device };
        for database in databases {
            location.check_file(database, true)?;
            for suffix in ["-journal", "-wal", "-shm"] {
                let mut sidecar = database.as_os_str().to_owned();
                sidecar.push(suffix);
                location.check_file(Path::new(&sidecar), true)?;
            }
        }
        Ok(location)
    }

    fn check_database(&self, path: &Path) -> Result<()> {
        self.check_file(path, false)
    }

    fn check_file(&self, path: &Path, allow_missing: bool) -> Result<()> {
        use std::os::unix::fs::MetadataExt as _;

        let entry = match std::fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if allow_missing && error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        if !entry.is_file() || entry.nlink() != 1 || entry.dev() != self.device {
            return Err(format!("redirected local SQLite file: {}", path.display()).into());
        }
        Ok(())
    }
}

#[cfg(unix)]
fn plain_directory(path: &Path) -> Result<std::fs::Metadata> {
    if let Err(error) = std::fs::create_dir(path) {
        if error.kind() != std::io::ErrorKind::AlreadyExists { return Err(error.into()); }
    }
    let entry = std::fs::symlink_metadata(path)?;
    if !entry.is_dir() {
        return Err(format!("redirected local SQLite directory: {}", path.display()).into());
    }
    Ok(entry)
}

#[cfg(not(unix))]
impl SqliteLocation {
    fn prepare(_working: &Path, _cst: &Path, _databases: [&Path; 2]) -> Result<Self> {
        Err("project-local SQLite filesystem identity is supported only on Unix".into())
    }

    fn check_database(&self, _path: &Path) -> Result<()> {
        unreachable!("SQLite location not supported")
    }
}
