//! Project-local source selection and recovery. Compiler queries and the
//! public current.json view are not stored here.
use crate::source_preparation::{Files, PreparedSources};
use crate::source_recovery::{trace_reachable, ClientSourceLimits, ClientSourceUsage, RetainedSources};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use std::path::{Path, PathBuf};
use wosy_protocol::{source_preparation_parts, Delta, FileEntry, FileKey, Kind, Node, NodeId, SourceConfig};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

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

struct SavedGeneration {
    source: PreparedSources,
    usage: ClientSourceUsage,
    config: SourceConfig,
    files: Vec<NodeId>,
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
            (1, 1) => {}
            _ => return Err("incompatible local source state schema".into()),
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
        let mut previous = before.selected
            .map(|root| self.load(target, root)).transpose()?;
        if let Some(previous) = previous.as_mut() {
            let staged = previous.stage_refresh(&self.project, Some(target))?;
            let (parsed, reused) = staged.observation_counts();
            let update = staged.submission();
            self.record(target, before, update.root, update.added, parsed, reused)
        } else {
            let initial = RetainedSources::observe(&self.project, Some(target), self.limits)?;
            let source = initial.selected();
            self.record(target, before, source.root, Some(&source.delta), source.parsed, source.reused)
        }
    }

    fn record(
        &mut self, target: &str, prior: SourceSelection, root: NodeId, added: Option<&Delta>,
        parsed: usize, reused: usize,
    ) -> Result<SourceStage> {
        let tx = self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if read_selection(&tx, target)? != prior {
            return Err("local source selection advanced during observation".into());
        }
        let pending = if prior.selected != Some(root) {
            let generation = prior.generation.checked_add(1).ok_or("local source generation exhausted")?;
            Some(SourceAttempt::new(root, generation))
        } else { None };
        let (mut new_nodes, mut new_bytes) = (0, 0);
        if let Some(attempt) = pending {
            if let Some(delta) = added {
                let mut insert = tx.prepare(
                    "INSERT INTO cst.nodes (digest, payload) VALUES (?1, ?2) ON CONFLICT(digest) DO NOTHING",
                )?;
                for (id, bytes) in &delta.nodes {
                    if insert.execute(params![id.0, bytes])? == 1 {
                        new_nodes += 1;
                        new_bytes += bytes.len();
                    }
                }
            }
            tx.execute(
                "INSERT INTO source_targets (target, pending, generation) VALUES (?1, ?2, ?3)
                 ON CONFLICT(target) DO UPDATE SET pending = excluded.pending, generation = excluded.generation",
                params![target, attempt.root.0, attempt.generation],
            )?;
        }
        tx.commit()?;
        Ok(SourceStage { root, parsed, reused, new_nodes, new_bytes, pending })
    }

    /// A complete resubmission for a reopened client and an empty or reopened
    /// daemon cache. This does not read the live workspace or invoke a parser.
    pub fn replay(&self, target: &str, which: ReplaySource) -> Result<Option<SourceReplay>> {
        let state = self.selection(target)?;
        let (root, attempt) = match which {
            ReplaySource::Selected => (state.selected, None),
            ReplaySource::Pending => (state.pending.map(SourceAttempt::root), state.pending),
        };
        root.map(|root| self.read_immutable(target, root).map(|saved| SourceReplay {
            source: saved.source, usage: saved.usage, attempt,
        })).transpose()
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

    fn read_immutable(&self, target: &str, root: NodeId) -> Result<SavedGeneration> {
        let mut nodes = Delta::default();
        let mut get = self.db.prepare("SELECT payload FROM cst.nodes WHERE digest = ?1")?;
        let closure = trace_reachable::<Vec<u8>, Box<dyn std::error::Error + Send + Sync>>(
            root, self.limits, |id| {
                let bytes: Vec<u8> = get.query_row([id.0], |row| row.get(0)).optional()?
                    .ok_or("persisted source generation is not closed")?;
                if Node::id(&bytes) != id { return Err("persisted source node hash mismatch".into()); }
                Ok(bytes)
            }, |id, bytes| { nodes.nodes.insert(id, bytes); },
        )?;
        let top = decode_node(&nodes, root)?;
        let parts = source_preparation_parts(&top).ok_or("unsupported persisted source schema")?;
        let config = decode_node(&nodes, parts.config)?;
        if config.kind != Kind::Config { return Err("expected persisted source configuration".into()); }
        let config: SourceConfig = serde_json::from_slice(&config.data)?;
        if config.target != target || top.data != config.schema()
            || config.parser.schema.0 != *wosy_syntax::base_cst_schema()
            || config.namespaces.get(&config.entry.package).is_none_or(|ns| {
                ns.git.is_some() || Path::new(&ns.project) != self.project
            })
        {
            return Err("persisted source does not match this project, target, or parser".into());
        }
        Ok(SavedGeneration {
            source: PreparedSources { root, delta: nodes, parsed: 0, reused: 0 },
            usage: closure.usage(), config, files: parts.files.to_vec(),
        })
    }

    fn load(&self, target: &str, root: NodeId) -> Result<RetainedSources> {
        let SavedGeneration { source, usage, config, files: entries } = self.read_immutable(target, root)?;
        let parser = config.parser.id();
        let mut files = Files::new();
        for id in entries {
            let file = FileEntry::from_node(id, decode_node(&source.delta, id)?)?;
            if file.metadata.policy != parser
                || files.insert(FileKey::from(&file.metadata.source), file).is_some()
            {
                return Err("invalid persisted source file index".into());
            }
        }
        if !files.contains_key(&config.entry) { return Err("persisted entry file is missing".into()); }
        Ok(RetainedSources::from_persisted(source, files, usage, self.limits))
    }
}

fn decode_node(delta: &Delta, id: NodeId) -> Result<Node> {
    delta.nodes.get(&id).and_then(|bytes| Node::decode(bytes))
        .ok_or_else(|| "invalid persisted source record".into())
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
