//! Internal source-preparation store and rooted compiler diagnostic operations.
use rowan::GreenNodeBuilder;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use surrealkv::{Durability, Transaction, Tree, TreeBuilder};
use wosy_project::{namespace_module_path, PackageRelativePath};
use wosy_protocol::{
    imports_from_cst, source_preparation_parts, source_preparation_schema, syntax_kind, token_text, walk_cst, CstEvent,
    Delta, FileEntry,
    FileKey, Import, InstalledGitSelection, Kind, Node, NodeId, SourceConfig, SourceUpdate,
};
use wosy_syntax::{
    typed_comment_from_token, AuthoredSpanMap, CanonicalCstRoot, CstNode, SourceIdentity, SyntaxKind,
    TypedComment,
};

const NODES: u8 = 1;
const INTERN: u8 = 2;
const EDGES_OUT: u8 = 3;
const EDGES_IN: u8 = 4;
const SNAPSHOT_ROOTS: u8 = 5;
const NAMESPACE_QUERIES: u8 = 11;
const FILE_AUTHORITY: u8 = 12;

fn key(tag: u8, node: NodeId) -> [u8; 33] {
    let mut key = [0; 33];
    key[0] = tag;
    key[1..].copy_from_slice(&node.0);
    key
}

fn check_source_marker(root: NodeId, marker: &[u8]) -> Result<()> {
    if marker != root.0 { return Err("published source marker does not match its root".into()); }
    Ok(())
}

fn edge(tag: u8, from: NodeId, kind: Edge, to: NodeId) -> [u8; 66] {
    let mut key = [0; 66];
    key[0] = tag;
    key[1..33].copy_from_slice(&from.0);
    key[33] = kind as u8;
    key[34..].copy_from_slice(&to.0);
    key
}

type Error = Box<dyn std::error::Error + Send + Sync>;
type Result<T> = std::result::Result<T, Error>;

mod base_declarations;
mod build_receipt;
mod build_session;
mod framed_session;
#[cfg(all(target_os = "linux", feature = "native-diagnostics"))]
mod native_diagnostics;
#[cfg(all(target_os = "linux", feature = "native-diagnostics"))]
pub use native_diagnostics::NativeKnownDiagnostics;
mod grammar_receipts;
mod installed_git_scope;
mod derivation_image;
mod pins;
mod project_declarations;
mod prepared_project;
mod rooted_llvm;
mod retention_plan;
mod rooted_llvm_receipts;
mod source_impact;
mod source_proposal;
mod source_reconciliation;
mod storage_worker;
pub use base_declarations::{BaseDeclarationsView, QueryDisposition};
pub use build_session::{BuildDiagnosticSession, BuildSourceOpening};
pub use framed_session::BuildSessionEnd;
pub use installed_git_scope::InstalledGitScopeRejection;
pub use wosy_protocol::{encode_session_frame, SessionFrameError, SessionFrameReader, MAX_SESSION_FRAME_BYTES};
pub use pins::SourcePin;
pub use project_declarations::{
    PreparedBaseModule, ProjectDeclarationImport, ProjectDeclarationModule, ProjectDeclarationReport,
    ProjectDeclarationsView,
};
pub use prepared_project::{
    BuildAnalysisKey, ProjectPreparationReport, RootedBuildAnalysis, RootedPreparedProject,
};
pub use rooted_llvm::{RootedLlvmIdentity, RootedLlvmOrigin, RootedLlvmPartition, RootedLlvmProjection, RootedLlvmRejection, RootedLlvmUnsupported};
pub use retention_plan::{SourceCasInventory, SourceRetentionInventory, SourceRetentionPlan};
pub use source_impact::{SourceFileChange, SourceImpact, SourceImpactLimits};
pub use source_proposal::{BaseCstAdmission, BaseCstAdmissionLimits, PreparedBaseIdentity, PreparedBaseDeclarations};
pub use source_reconciliation::SourceReconciliation;
pub use storage_worker::{RetentionRead, RetentionReadJob, SourceStorageWorker, StorageAdmissionError};
pub use wosy_protocol::{BuildAnalysisOrigin, BuildAnalysisPhase};

pub struct SourceStore {
    tree: Tree,
    blobs: PathBuf,
    base_flights: base_declarations::Flights,
    project_commit: tokio::sync::Mutex<()>,
    prepared_projects: prepared_project::PreparationCache,
    source_reads: retention_plan::SourceReads,
    grammar_receipts: grammar_receipts::GrammarReceipts,
    llvm_receipts: rooted_llvm_receipts::ExecutableLlvmReceipts,
    llvm_flights: base_declarations::Flights<rooted_llvm_receipts::ExecutableLlvmKey, rooted_llvm_receipts::ExecutableLlvmFlight>,
    #[cfg(test)]
    test_without_derivation_image: bool,
}

#[derive(Default)]
pub struct PublishReport {
    pub written: usize,
    pub namespace_derived: usize,
    pub namespace_reused: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourcePublicationMode {
    Existing,
    Full,
    Incremental,
    Recovered,
}

pub struct SourcePublication {
    pub mode: SourcePublicationMode,
    pub report: PublishReport,
    pub conflicts: usize,
}

#[derive(Clone, Copy)]
enum SourcePayload<'a> {
    Full(&'a Delta),
    Staged(SourceUpdate<'a>),
}

impl SourceStore {
    pub fn open(dir: &Path, block_cache_bytes: u64) -> Result<Self> {
        let tree = TreeBuilder::new()
            .with_path(dir.to_owned())
            .with_block_cache_capacity(block_cache_bytes)
            .with_versioning(false, 0)
            .with_enable_vlog(false)
            .build()?;
        Ok(Self {
            tree, blobs: dir.join("cas"), base_flights: Default::default(),
            project_commit: Default::default(),
            prepared_projects: Default::default(), source_reads: Default::default(),
            grammar_receipts: Default::default(),
            llvm_receipts: Default::default(),
            llvm_flights: Default::default(),
            #[cfg(test)]
            test_without_derivation_image: false,
        })
    }

    pub async fn close(self) -> Result<()> {
        self.tree.close().await?;
        Ok(())
    }

    /// A race restarts publication against a complete committed snapshot.
    pub async fn publish(&self, root: NodeId, delta: &Delta) -> Result<PublishReport> {
        Ok(self.publish_source(root, SourcePayload::Full(delta)).await?.report)
    }

    /// Admit and publish a previously staged speculative source refresh.
    /// Recover only its requested root if the predecessor was lost. This
    /// direct operation does not confer a transport or retention capability.
    pub async fn publish_update(&self, update: SourceUpdate<'_>) -> Result<SourcePublication> {
        self.publish_source(update.root, SourcePayload::Staged(update)).await
    }

    async fn publish_source(&self, root: NodeId, input: SourcePayload<'_>) -> Result<SourcePublication> {
        use SourcePublicationMode::{Existing, Full, Incremental, Recovered};
        let mut conflicts = 0;
        loop {
            let mut tx = self.tree.begin()?;
            tx.set_durability(Durability::Eventual);
            if tx.get(&key(SNAPSHOT_ROOTS, root))?.is_some() {
                return Ok(SourcePublication { mode: Existing, report: PublishReport::default(), conflicts });
            }
            let (mode, added, base) = match input {
                SourcePayload::Full(delta) => (Full, Some(delta), None),
                SourcePayload::Staged(update) => {
                    // If both roots are the same, the first read proved it
                    // absent. Other predecessor reads join commit validation.
                    let published = update.base != root
                        && tx.get_for_update(&key(SNAPSHOT_ROOTS, update.base))?.is_some()
                        && source_top(&tx, update.base)?.is_some();
                    if published {
                        (Incremental, update.added, None)
                    } else {
                        (Recovered, update.added, Some(update.base_payload))
                    }
                }
            };
            let written = intern_closure(&mut tx, root, |id| {
                added.and_then(|delta| delta.nodes.get(&id))
                    .or_else(|| base.and_then(|delta| delta.nodes.get(&id))).map(Vec::as_slice)
            })?;
            let mut report = PublishReport { written, ..Default::default() };
            validate_snapshot(&mut tx, root, &mut report)?;
            tx.set(&key(SNAPSHOT_ROOTS, root), &root.0)?;
            match tx.commit().await {
                Ok(()) => return Ok(SourcePublication { mode, report, conflicts }),
                Err(surrealkv::Error::TransactionWriteConflict) => conflicts += 1,
                Err(error) => return Err(error.into()),
            }
        }
    }

    pub fn snapshot(&self, root: NodeId) -> Result<Option<SourceView>> {
        let Some((tx, read)) = self.begin_source_read(root)? else { return Ok(None); };
        self.snapshot_with_read(root, tx, read)
    }

    fn snapshot_with_read(
        &self, root: NodeId, tx: Transaction, read: retention_plan::SourceRead,
    ) -> Result<Option<SourceView>> {
        let Some(top) = source_top(&tx, root)? else { return Ok(None); };
        let index = read_index(&tx, top)?;
        Ok(Some(SourceView {
            tx,
            root,
            executable: index.executable,
            dependencies: index.dependencies,
            annotation: index.annotation,
            config: index.config,
            modules: index.modules,
            module_edges: index.module_edges,
            files: index.files,
            read,
        }))
    }

    fn blob_path(&self, digest: NodeId) -> PathBuf {
        self.blobs.join(digest.hex())
    }
}

fn published_source(tx: &Transaction, root: NodeId) -> Result<Option<Node>> {
    if tx.get(&key(SNAPSHOT_ROOTS, root))?.is_none() {
        return Ok(None);
    }
    source_top(tx, root)
}

fn source_top(tx: &Transaction, root: NodeId) -> Result<Option<Node>> {
    let top = read_node(tx, root)?;
    Ok(supported_top(&top).then_some(top))
}

fn intern_closure<'a>(
    tx: &mut Transaction, root: NodeId, lookup: impl Fn(NodeId) -> Option<&'a [u8]>,
) -> Result<usize> {
    let mut work = vec![(root, Expected::Exact(Kind::SourcePreparation))];
    let mut visited = HashMap::new();
    let mut inserted = 0;
    while let Some((id, expected)) = work.pop() {
        if let Some(&kind) = visited.get(&id) {
            expected.check(kind)?;
            continue;
        }
        if tx.get(&key(INTERN, id))?.is_some() {
            let kind = read_node(tx, id)?.kind;
            expected.check(kind)?;
            visited.insert(id, kind);
            continue;
        }
        let bytes = lookup(id).ok_or("source delta is not closed")?;
        if Node::id(bytes) != id {
            return Err("source delta node hash mismatch".into());
        }
        let node = Node::decode(bytes).ok_or("invalid source delta node encoding")?;
        expected.check(node.kind)?;
        validate_shape(&node)?;
        visited.insert(id, node.kind);
        write_node(tx, id, bytes)?;
        for (i, &child) in node.children.iter().enumerate() {
            let (role, expected) = child_edge(node.kind, i);
            write_edge(tx, id, role, child)?;
            work.push((child, expected));
        }
        inserted += 1;
    }
    Ok(inserted)
}

fn write_node(tx: &mut Transaction, id: NodeId, bytes: &[u8]) -> Result<()> {
    tx.set(&key(NODES, id), bytes)?;
    tx.set(&key(INTERN, id), &id.0)?;
    Ok(())
}

fn write_edge(tx: &mut Transaction, from: NodeId, role: Edge, to: NodeId) -> Result<()> {
    tx.set(&edge(EDGES_OUT, from, role, to), &[])?;
    tx.set(&edge(EDGES_IN, to, role, from), &[])?;
    Ok(())
}

#[derive(Clone, Copy)]
enum Expected {
    Cst,
    Exact(Kind),
}

impl Expected {
    fn check(self, actual: Kind) -> Result<()> {
        let good = match self {
            Self::Cst => matches!(actual, Kind::Syntax | Kind::Token),
            Self::Exact(kind) => kind == actual,
        };
        if good { Ok(()) } else { Err("incompatible source graph edge".into()) }
    }
}

fn validate_shape(node: &Node) -> Result<()> {
    use Kind::*;
    let count = node.children.len();
    let good = match node.kind {
        Token => count == 0 && syntax_kind(node).is_ok() && token_text(node).is_ok(),
        Syntax => node.data.len() == 2 && syntax_kind(node).is_ok(),
        File => count == 3,
        Module => module_parts(node).is_ok(),
        Config | Annotations => count == 0,
        ExecutableFile | AnnotationFile => count == 1,
        Executables | AnnotationTree | Dependencies => node.data.is_empty(),
        SourcePreparation => supported_top(node),
        NamespaceFacts | BaseDeclarations => count == 1,
        ModuleDeclarations => count >= 2,
        BuildDiagnosticReceipt => count == 4,
    };
    if good { Ok(()) } else { Err("invalid base source schema or node shape".into()) }
}

fn supported_top(node: &Node) -> bool {
    source_preparation_parts(node).is_some()
}

#[derive(Clone, Copy)]
#[repr(u8)]
enum Edge {
    CstChild = 1,
    Authored,
    Executable,
    Annotation,
    Config,
    Files,
    Dependencies,
    ProjectionEntry,
    LocalExecutable,
    Imports,
    NamespaceSource,
    NamespaceAuthority,
    DeclarationSource,
    ModuleDeclarationSource,
    ModuleDeclarationBody,
    ModuleDeclarationImport,
    ReceiptDeclarations,
}

fn child_edge(kind: Kind, i: usize) -> (Edge, Expected) {
    use Expected::Exact;
    use Kind::*;
    match kind {
        Syntax => (Edge::CstChild, Expected::Cst),
        File => match i {
            0 => (Edge::Authored, Exact(Syntax)),
            1 => (Edge::Executable, Exact(Syntax)),
            _ => (Edge::Annotation, Exact(Annotations)),
        },
        ExecutableFile => (Edge::Executable, Exact(Syntax)),
        AnnotationFile => (Edge::Annotation, Exact(Annotations)),
        Executables => (Edge::ProjectionEntry, Exact(ExecutableFile)),
        AnnotationTree => (Edge::ProjectionEntry, Exact(AnnotationFile)),
        Dependencies => (Edge::ProjectionEntry, Exact(Module)),
        Module if i == 0 => (Edge::LocalExecutable, Exact(ExecutableFile)),
        Module => (Edge::Imports, Exact(Module)),
        SourcePreparation => match i {
            0 => (Edge::Config, Exact(Config)),
            1 => (Edge::Executable, Exact(Executables)),
            2 => (Edge::Annotation, Exact(AnnotationTree)),
            3 => (Edge::Dependencies, Exact(Dependencies)),
            _ => (Edge::Files, Exact(File)),
        },
        NamespaceFacts => (Edge::NamespaceSource, Exact(Syntax)),
        BaseDeclarations => (Edge::DeclarationSource, Exact(Syntax)),
        ModuleDeclarations => match i {
            0 => (Edge::ModuleDeclarationSource, Exact(Module)),
            1 => (Edge::ModuleDeclarationBody, Exact(BaseDeclarations)),
            _ => (Edge::ModuleDeclarationImport, Exact(ModuleDeclarations)),
        },
        BuildDiagnosticReceipt => match i {
            0 => (Edge::Config, Exact(Config)),
            1 => (Edge::Executable, Exact(Executables)),
            2 => (Edge::Dependencies, Exact(Dependencies)),
            _ => (Edge::ReceiptDeclarations, Exact(ModuleDeclarations)),
        },
        Token | Annotations | Config => unreachable!("validated leaf"),
    }
}

fn read_node(tx: &Transaction, id: NodeId) -> Result<Node> {
    read_node_with(tx, id, |_| Ok(()))
}

fn read_node_with(tx: &Transaction, id: NodeId, charge: impl FnOnce(usize) -> Result<()>) -> Result<Node> {
    read_node_using(tx, id, |bytes| charge(bytes.len()))
}

fn read_node_using(tx: &Transaction, id: NodeId, inspect: impl FnOnce(&[u8]) -> Result<()>) -> Result<Node> {
    let bytes = tx.get(&key(NODES, id))?.ok_or("referenced source node is missing")?;
    inspect(&bytes)?;
    Node::decode(&bytes).ok_or_else(|| "invalid stored source node".into())
}

fn read_verified_node_with(tx: &Transaction, id: NodeId, charge: impl FnOnce(usize) -> Result<()>) -> Result<Node> {
    read_node_using(tx, id, |bytes| {
        charge(bytes.len())?;
        if Node::id(bytes) != id { return Err("stored source node differs from its rooted identity".into()); }
        Ok(())
    })
}

struct Index {
    config: NodeId,
    executable: NodeId,
    dependencies: NodeId,
    annotation: NodeId,
    files: BTreeMap<FileKey, FileEntry>,
    modules: BTreeMap<FileKey, NodeId>,
    executable_paths: BTreeMap<FileKey, (NodeId, NodeId)>,
    annotation_paths: BTreeMap<FileKey, (NodeId, NodeId)>,
    module_edges: BTreeMap<FileKey, Vec<NodeId>>,
}

fn unique<K: Ord, V>(entries: &mut BTreeMap<K, V>, key: K, value: V) -> Result<()> {
    use std::collections::btree_map::Entry;
    match entries.entry(key) {
        Entry::Vacant(entry) => { entry.insert(value); Ok(()) }
        Entry::Occupied(_) => Err("duplicate path in source index".into()),
    }
}

fn module_parts(node: &Node) -> Result<(NodeId, &[NodeId])> {
    let [local, imports @ ..] = node.children.as_slice() else {
        return Err("missing local module executable".into());
    };
    if node.kind != Kind::Module || !node.data.is_empty() { return Err("invalid source module shape".into()); }
    Ok((*local, imports))
}

fn projection_file_parts(node: &Node, kind: Kind) -> Result<(FileKey, NodeId)> {
    let [child] = node.children.as_slice() else { return Err("invalid projection leaf".into()); };
    Expected::Exact(kind).check(node.kind)?;
    Ok((serde_json::from_slice(&node.data)?, *child))
}

fn read_paths_using(
    read: &mut impl FnMut(NodeId) -> Result<Node>, root: NodeId, leaf_kind: Kind,
) -> Result<BTreeMap<FileKey, (NodeId, NodeId)>> {
    let mut paths = BTreeMap::new();
    for id in read(root)?.children {
        let (key, child) = projection_file_parts(&read(id)?, leaf_kind)?;
        unique(&mut paths, key, (id, child))?;
    }
    Ok(paths)
}

fn read_index(tx: &Transaction, top: Node) -> Result<Index> {
    read_index_charged(tx, top, |_| Ok(()))
}

fn read_index_charged(
    tx: &Transaction, top: Node, charge_imports: impl FnMut(usize) -> Result<()>,
) -> Result<Index> {
    read_index_using(top, |id| read_node(tx, id), charge_imports)
}

fn read_index_using(
    top: Node, mut read: impl FnMut(NodeId) -> Result<Node>, mut charge_imports: impl FnMut(usize) -> Result<()>,
) -> Result<Index> {
    let parts = source_preparation_parts(&top).ok_or("incomplete source preparation")?;
    let mut entries = BTreeMap::new();
    for &id in parts.files {
        let entry = FileEntry::from_node(id, read(id)?)?;
        unique(&mut entries, FileKey::from(&entry.metadata.source), entry)?;
    }
    let executable_paths = read_paths_using(&mut read, parts.executable, Kind::ExecutableFile)?;
    let annotation_paths = read_paths_using(&mut read, parts.annotation, Kind::AnnotationFile)?;
    let by_projection: BTreeMap<_, _> = executable_paths.iter().map(|(key, (id, _))| (*id, key)).collect();
    let mut modules = BTreeMap::new();
    let mut module_edges = BTreeMap::new();
    for id in read(parts.dependencies)?.children {
        let node = read(id)?;
        let (local, imports) = module_parts(&node)?;
        charge_imports(imports.len())?;
        let key = *by_projection.get(&local).ok_or("module outside executable paths")?;
        unique(&mut modules, key.clone(), id)?;
        module_edges.insert(key.clone(), imports.to_vec());
    }
    Ok(Index {
        config: parts.config, executable: parts.executable, dependencies: parts.dependencies, annotation: parts.annotation,
        files: entries, modules, executable_paths, annotation_paths, module_edges,
    })
}

fn check_path(path: &PackageRelativePath) -> Result<()> {
    PackageRelativePath::new(path.as_path().to_owned())?;
    Ok(())
}

fn validate_snapshot(tx: &mut Transaction, root: NodeId, report: &mut PublishReport) -> Result<()> {
    validate_snapshot_with(tx, root, |_| Ok(()), |tx, file, categories| {
        admit_namespace_authority(tx, file, categories, report)
    }).map(|_| ())
}

fn validate_snapshot_with(
    tx: &mut Transaction, root: NodeId, check_scope: impl FnOnce(&SourceConfig) -> Result<()>,
    mut check_file: impl FnMut(&mut Transaction, &FileEntry, &[String]) -> Result<()>,
) -> Result<(Index, SourceConfig)> {
    // The root's closure has already passed the schema boundary.
    let top = read_node(tx, root)?;
    let schema = top.data.as_slice().try_into().map(|array: &[u8; 32]| *array)?;
    let index = read_index(tx, top)?;
    let config: SourceConfig = serde_json::from_slice(&read_node(tx, index.config)?.data)?;
    if &schema != config.schema()
        || config.namespaces.get(&config.entry.package).is_none_or(|ns| ns.git.is_some())
        || config.parser.schema.0 != *wosy_syntax::base_cst_schema()
        || !index.files.contains_key(&config.entry)
        || index.files.len() != index.modules.len()
        || index.files.len() != index.executable_paths.len()
        || index.files.len() != index.annotation_paths.len()
    {
        return Err("incompatible source configuration or index".into());
    }
    check_scope(&config)?;
    for ns in config.namespaces.values() {
        if let Some(path) = &ns.source_root {
            check_path(path)?;
        }
    }
    let policy = config.parser.id();
    let mut reachable = BTreeSet::new();
    let mut pending = vec![config.entry.clone()];
    while let Some(key) = pending.pop() {
        if !reachable.insert(key.clone()) {
            continue;
        }
        check_path(&key.path)?;
        let file = index.files.get(&key).ok_or("missing file provenance")?;
        let source = &file.metadata;
        let namespace = config.namespaces.get(&key.package).ok_or("unknown source namespace")?;
        if source.source.project != namespace.project || source.policy != policy
            || index.executable_paths.get(&key).map(|x| x.1) != Some(file.executable)
            || index.annotation_paths.get(&key).map(|x| x.1) != Some(file.annotation)
        {
            return Err("source provenance does not match the projections".into());
        }
        check_file(tx, file, &config.parser.categories)?;
        // The file's imports now have exact canonical CST authority, including
        // their authored positions. Use the same verified order for module edges.
        let edges = index.module_edges.get(&key).ok_or("missing module for source")?;
        if source.imports.len() != edges.len() {
            return Err("module edges do not match source imports".into());
        }
        for (import, dependency) in source.imports.iter().zip(edges) {
            let ns = config.namespaces.get(&import.origin).ok_or("unknown import namespace")?;
            let imported = FileKey {
                package: import.origin.clone(),
                path: namespace_module_path(ns.source_root.as_ref(), &import.path)?.into_owned(),
            };
            if index.modules.get(&imported) != Some(dependency)
                || (imported.package == config.entry.package && source.source.project != ns.project)
            {
                return Err("module edge points at the wrong resolved source".into());
            }
            pending.push(imported);
        }
    }
    if reachable.len() != index.files.len() {
        return Err("source index contains unreachable files".into());
    }
    Ok((index, config))
}

fn read_comments(tx: &Transaction, id: NodeId) -> Result<Vec<TypedComment>> {
    Ok(serde_json::from_slice(&read_node(tx, id)?.data)?)
}

fn authority_key(tag: u8, source: NodeId) -> [u8; 65] {
    let mut bytes = [0; 65];
    bytes[0] = tag;
    bytes[1..33].copy_from_slice(source_preparation_schema());
    bytes[33..].copy_from_slice(&source.0);
    bytes
}

fn admit_namespace_authority(
    tx: &mut Transaction,
    file: &FileEntry,
    configured: &[String],
    report: &mut PublishReport,
) -> Result<()> {
    let provenance = authority_key(FILE_AUTHORITY, file.file);
    if tx.get(&provenance)?.is_some() {
        return Ok(());
    }
    let query = authority_key(NAMESPACE_QUERIES, file.executable);
    let existing = tx.get(&query)?;
    let tree = if existing.is_some() { FileTree::None } else { FileTree::Executable };
    let checked = validate_file(tx, file, configured, tree, None)?;
    finish_namespace_authority(tx, file, provenance, query, existing, checked, report)
}

fn admit_checked_namespace_authority(
    tx: &mut Transaction, file: &FileEntry, checked: CheckedFile, report: &mut PublishReport,
) -> Result<()> {
    let provenance = authority_key(FILE_AUTHORITY, file.file);
    if tx.get(&provenance)?.is_some() { return Ok(()); }
    let query = authority_key(NAMESPACE_QUERIES, file.executable);
    let existing = tx.get(&query)?;
    finish_namespace_authority(tx, file, provenance, query, existing, checked, report)
}

fn finish_namespace_authority(
    tx: &mut Transaction, file: &FileEntry, provenance: [u8; 65], query: [u8; 65],
    existing: Option<surrealkv::Value>, checked: CheckedFile, report: &mut PublishReport,
) -> Result<()> {
    let (fact, mut imports) = if let Some(id) = existing {
        let id = NodeId(id.as_slice().try_into()?);
        let facts = read_node(tx, id)?;
        let imports = serde_json::from_slice(&facts.data)?;
        report.namespace_reused += 1;
        (id, imports)
    } else {
        let imports = imports_from_cst(&checked.executable.ok_or("missing checked executable tree")?)?;
        let facts = Node {
            kind: Kind::NamespaceFacts,
            data: serde_json::to_vec(&imports)?,
            children: vec![file.executable],
        };
        let bytes = facts.encode();
        let id = Node::id(&bytes);
        if tx.get(&key(INTERN, id))?.is_none() {
            write_node(tx, id, &bytes)?;
            write_edge(tx, id, Edge::NamespaceSource, file.executable)?;
            report.written += 1;
        }
        tx.set(&query, &id.0)?;
        report.namespace_derived += 1;
        (id, imports)
    };
    restore_authored_spans(&mut imports, &checked.comments);
    if imports != file.metadata.imports {
        return Err("source namespace hints differ from canonical declarations".into());
    }
    tx.set(&provenance, &fact.0)?;
    write_edge(tx, file.file, Edge::NamespaceAuthority, fact)?;
    Ok(())
}

fn restore_authored_spans(imports: &mut [Import], comments: &[TypedComment]) {
    if comments.is_empty() || imports.is_empty() {
        return;
    }
    let authored = AuthoredSpanMap::from_comments(comments);
    for import in imports {
        let span = authored.project(wosy_syntax::ByteSpan::new(import.span.start, import.span.end));
        import.span = wosy_project::ByteSpan::new(span.start, span.end);
    }
}

#[derive(Clone, Copy, PartialEq)]
enum FileTree {
    None,
    Executable,
    Authored,
    Both,
}

struct CheckedFile {
    authored: Option<CstNode>,
    executable: Option<CstNode>,
    comments: Vec<TypedComment>,
}

fn validate_file(
    tx: &Transaction, file: &FileEntry, configured: &[String], tree: FileTree,
    work: Option<&mut source_proposal::CstWork>,
) -> Result<CheckedFile> {
    validate_file_using(file, configured, tree, work, |id, charge| {
        read_node_with(tx, id, |bytes| charge.map_or(Ok(()), |work| work.read(bytes)))
    })
}

fn validate_file_using(
    file: &FileEntry, configured: &[String], tree: FileTree,
    mut work: Option<&mut source_proposal::CstWork>,
    mut read: impl FnMut(NodeId, Option<&mut source_proposal::CstWork>) -> Result<Node>,
) -> Result<CheckedFile> {
    let bounded = work.is_some();
    if let Some(work) = work.as_mut() {
        work.file(file.metadata.bytes as usize)?;
    }
    let comments: Vec<TypedComment> = serde_json::from_slice(&read(file.annotation, work.as_deref_mut())?.data)?;
    let mut position = 0u32;
    let mut hasher = blake3::Hasher::new();
    let mut next_comment = comments.iter();
    let authored = read(file.authored, work.as_deref_mut())?;
    if authored.kind != Kind::Syntax || syntax_kind(&authored)? != SyntaxKind::Root {
        return Err("invalid canonical Rowan root".into());
    }
    let same_projection = tree == FileTree::Both && comments.is_empty();
    let mut authored_builder = matches!(tree, FileTree::Authored | FileTree::Both).then(GreenNodeBuilder::new);
    let mut executable_builder = (matches!(tree, FileTree::Executable | FileTree::Both) && !same_projection)
        .then(GreenNodeBuilder::new);
    enum Step {
        Enter(NodeId, Node, Option<NodeId>, usize),
        Children(std::vec::IntoIter<NodeId>, std::vec::IntoIter<NodeId>, usize),
    }
    let mut pending = vec![Step::Enter(file.authored, authored, Some(file.executable), 1)];
    while let Some(step) = pending.pop() {
        let (id, node, projection, depth) = match step {
            Step::Enter(id, node, projection, depth) => (id, node, projection, depth),
            Step::Children(mut children, mut projected, depth) => {
                let Some(child) = children.next() else {
                    if projected.next().is_some() { return Err("extra executable syntax nodes".into()); }
                    for builder in [&mut authored_builder, &mut executable_builder].into_iter().flatten() {
                        builder.finish_node();
                    }
                    continue;
                };
                let record = read(child, work.as_deref_mut())?;
                let projection = if record.kind == Kind::Token && syntax_kind(&record)? == SyntaxKind::TypedComment {
                    None
                } else {
                    Some(projected.next().ok_or("incomplete executable syntax projection")?)
                };
                // Read one child at a time; don't clone all sibling payloads
                // before charging their unfolded bytes.
                pending.push(Step::Children(children, projected, depth));
                (child, record, projection, depth)
            }
        };
        match node.kind {
            Kind::Syntax => {
                if let Some(work) = work.as_mut() { work.children(depth, node.children.len())?; }
                if authored_builder.is_some() || executable_builder.is_some() {
                    let kind = rowan::SyntaxKind(syntax_kind(&node)? as u16);
                    for builder in [&mut authored_builder, &mut executable_builder].into_iter().flatten() {
                        builder.start_node(kind);
                    }
                }
                let projected = if projection == Some(id) {
                    node.children.clone()
                } else {
                    let projected = read(projection.ok_or("missing CST projection")?, work.as_deref_mut())?;
                    if projected.kind != Kind::Syntax || projected.data != node.data {
                        return Err("incompatible executable syntax projection".into());
                    }
                    projected.children
                };
                pending.push(Step::Children(node.children.into_iter(), projected.into_iter(), depth + 1));
            }
            Kind::Token => {
                let text = token_text(&node)?;
                let end = position.checked_add(text.len().try_into()?).ok_or("CST exceeds the source range")?;
                if bounded && end > file.metadata.bytes {
                    return Err("CST exceeds its admitted authored byte range".into());
                }
                if projection.is_none() {
                    let canonical = typed_comment_from_token(text, position, configured)
                        .ok_or("invalid canonical typed comment token")?;
                    if next_comment.next() != Some(&canonical) {
                        return Err("typed comment does not match source provenance".into());
                    }
                } else if projection != Some(id) {
                    return Err("executable token differs from authored source".into());
                }
                if authored_builder.is_some() || (projection.is_some() && executable_builder.is_some()) {
                    let kind = rowan::SyntaxKind(syntax_kind(&node)? as u16);
                    if let Some(builder) = authored_builder.as_mut() { builder.token(kind, text); }
                    if let Some(builder) = executable_builder.as_mut().filter(|_| projection.is_some()) {
                        builder.token(kind, text);
                    }
                }
                hasher.update(text.as_bytes());
                position = end;
            }
            _ => return Err("invalid authored CST child".into()),
        }
    }
    if position != file.metadata.bytes || hasher.finalize().as_bytes() != &file.metadata.revision.0
        || next_comment.next().is_some()
    {
        return Err("file observation does not match its authored CST".into());
    }
    let authored = authored_builder.map(|tree| CstNode::new_root(tree.finish()));
    let executable = if same_projection { authored.clone() }
        else { executable_builder.map(|tree| CstNode::new_root(tree.finish())) };
    Ok(CheckedFile { authored, executable, comments })
}

pub struct SourceView {
    tx: Transaction,
    root: NodeId,
    pub executable: NodeId,
    pub annotation: NodeId,
    dependencies: NodeId,
    config: NodeId,
    pub modules: BTreeMap<FileKey, NodeId>,
    module_edges: BTreeMap<FileKey, Vec<NodeId>>,
    pub files: BTreeMap<FileKey, FileEntry>,
    read: retention_plan::SourceRead,
}

struct ConfiguredSourceView {
    source: SourceView,
    config: SourceConfig,
}

pub struct AuthoredFile {
    pub cst: CanonicalCstRoot,
    pub comments: Vec<TypedComment>,
}

fn file_source(file: &FileEntry) -> Result<SourceIdentity> {
    let source = &file.metadata.source;
    let path = source.path.as_path().to_str().ok_or("source path is not UTF-8")?;
    Ok(SourceIdentity::new(
        source.project.clone(), source.package.clone(), path.to_owned(), file.metadata.revision.hex(),
    ))
}

fn read_cst(tx: &Transaction, root: NodeId) -> Result<CstNode> {
    read_cst_with(tx, root, None)
}

fn read_cst_with(
    tx: &Transaction, root: NodeId, mut work: Option<&mut source_proposal::CstWork>,
) -> Result<CstNode> {
    if let Some(work) = work.as_mut() { work.start_tree()?; }
    let mut state = (work, GreenNodeBuilder::new());
    walk_cst(root, &mut state, |(work, _), id| {
        read_node_with(tx, id, |bytes| work.as_mut().map_or(Ok(()), |work| work.read(bytes)))
    }, |(work, green), event| {
        match event {
            CstEvent::Start { kind, depth, children } => {
                if let Some(work) = work { work.children(depth, children)?; }
                green.start_node(rowan::SyntaxKind(kind as u16));
            }
            CstEvent::Token { kind, text } => green.token(rowan::SyntaxKind(kind as u16), text),
            CstEvent::Finish => green.finish_node(),
        }
        Ok(())
    })?;
    Ok(CstNode::new_root(state.1.finish()))
}

fn read_source_config(tx: &Transaction, id: NodeId) -> Result<SourceConfig> {
    read_source_config_charged(tx, id, |_| Ok(()))
}

fn read_source_config_charged(
    tx: &Transaction, id: NodeId, charge: impl FnOnce(usize) -> Result<()>,
) -> Result<SourceConfig> {
    let node = read_verified_node_with(tx, id, charge)?;
    Expected::Exact(Kind::Config).check(node.kind)?;
    validate_shape(&node)?;
    Ok(serde_json::from_slice(&node.data)?)
}

impl SourceView {
    fn source_config(&self) -> Result<SourceConfig> {
        read_source_config(&self.tx, self.config)
    }

    fn configured(self) -> Result<ConfiguredSourceView> {
        let config = self.source_config()?;
        Ok(ConfiguredSourceView { source: self, config })
    }

    /// Describe the exact lock and patch selections of this rooted view,
    /// independently from the current workspace and its editable file bytes.
    pub fn installed_git_selections(&self) -> Result<BTreeMap<String, InstalledGitSelection>> {
        Ok(self.source_config()?.namespaces.into_iter().filter_map(|(name, namespace)| {
            namespace.git.map(|selection| (name, selection))
        }).collect())
    }

    /// Reconstitute the real Rowan tree from the selected root without reparsing.
    pub fn authored(&self, key: &FileKey) -> Result<AuthoredFile> {
        let file = self.files.get(key).ok_or("file outside selected source root")?;
        Ok(AuthoredFile {
            cst: CanonicalCstRoot::new(file_source(file)?, read_cst(&self.tx, file.authored)?),
            comments: read_comments(&self.tx, file.annotation)?,
        })
    }
}
