//! Internal source-preparation store. No socket or build/check query yet.
use rowan::GreenNodeBuilder;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use surrealkv::{Durability, Transaction, Tree, TreeBuilder};
use wosy_project::{namespace_module_path, PackageRelativePath};
use wosy_protocol::{
    imports_from_cst, installed_git_source_schema, source_preparation_schema, Delta, FileEntry,
    FileKey, Import, InstalledGitSelection, Kind, Node, NodeId, SourceConfig,
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
mod pins;
mod project_declarations;
mod prepared_project;
mod retention_plan;
mod storage_worker;
pub use base_declarations::{BaseDeclarationsView, QueryDisposition};
pub use pins::SourcePin;
pub use project_declarations::{
    ProjectDeclarationImport, ProjectDeclarationModule, ProjectDeclarationReport, ProjectDeclarationsView,
};
pub use prepared_project::{BuildAnalysisPhase, ProjectPreparationReport, RootedBuildAnalysis, RootedPreparedProject};
pub use retention_plan::{SourceCasInventory, SourceRetentionInventory, SourceRetentionPlan};
pub use storage_worker::{RetentionRead, RetentionReadJob, SourceStorageWorker, StorageAdmissionError};

pub struct SourceStore {
    tree: Tree,
    blobs: PathBuf,
    base_flights: base_declarations::Flights,
    project_commit: tokio::sync::Mutex<()>,
    prepared_projects: prepared_project::PreparationCache,
    source_reads: retention_plan::SourceReads,
}

#[derive(Default)]
pub struct PublishReport {
    pub written: usize,
    pub namespace_derived: usize,
    pub namespace_reused: usize,
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
        })
    }

    pub async fn close(self) -> Result<()> {
        self.tree.close().await?;
        Ok(())
    }

    /// A race restarts publication against a complete committed snapshot.
    pub async fn publish(&self, root: NodeId, delta: &Delta) -> Result<PublishReport> {
        loop {
            let mut tx = self.tree.begin()?;
            tx.set_durability(Durability::Eventual);
            if tx.get(&key(SNAPSHOT_ROOTS, root))?.is_some() {
                return Ok(PublishReport::default());
            }
            let mut report = PublishReport { written: intern_closure(&mut tx, root, delta)?, ..Default::default() };
            validate_snapshot(&mut tx, root, &mut report)?;
            tx.set(&key(SNAPSHOT_ROOTS, root), &root.0)?;
            match tx.commit().await {
                Ok(()) => return Ok(report),
                Err(surrealkv::Error::TransactionWriteConflict) => continue,
                Err(error) => return Err(error.into()),
            }
        }
    }

    pub fn snapshot(&self, root: NodeId) -> Result<Option<SourceView>> {
        let Some((tx, read)) = self.begin_source_read(root)? else { return Ok(None); };
        let Some(top) = source_top(&tx, root)? else { return Ok(None); };
        let index = read_index(&tx, top)?;
        Ok(Some(SourceView {
            tx,
            root,
            executable: index.executable,
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

fn intern_closure(
    tx: &mut Transaction, root: NodeId, delta: &Delta,
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
        let bytes = delta.nodes.get(&id).ok_or("source delta is not closed")?;
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
        Module => count >= 1 && node.data.is_empty(),
        Config | Annotations => count == 0,
        ExecutableFile | AnnotationFile => count == 1,
        Executables | AnnotationTree | Dependencies => node.data.is_empty(),
        SourcePreparation => supported_top(node),
        NamespaceFacts | BaseDeclarations => count == 1,
        ModuleDeclarations => count >= 2,
    };
    if good { Ok(()) } else { Err("invalid base source schema or node shape".into()) }
}

fn supported_top(node: &Node) -> bool {
    node.kind == Kind::SourcePreparation
        && node.children.len() >= 5
        && (node.data == source_preparation_schema() || node.data == installed_git_source_schema())
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
        Token | Annotations | Config => unreachable!("validated leaf"),
    }
}

fn read_node(tx: &Transaction, id: NodeId) -> Result<Node> {
    let bytes = tx.get(&key(NODES, id))?.ok_or("referenced source node is missing")?;
    Node::decode(&bytes).ok_or_else(|| "invalid stored source node".into())
}

fn syntax_kind(node: &Node) -> Result<SyntaxKind> {
    let raw = node.data.first_chunk::<2>().ok_or("missing syntax kind")?;
    SyntaxKind::try_from_raw(u16::from_be_bytes(*raw)).ok_or_else(|| "unknown syntax kind".into())
}

fn token_text(node: &Node) -> Result<&str> {
    Ok(std::str::from_utf8(node.data.get(2..).ok_or("missing token kind")?)?)
}

struct Index {
    config: NodeId,
    executable: NodeId,
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

fn read_paths(tx: &Transaction, root: NodeId) -> Result<BTreeMap<FileKey, (NodeId, NodeId)>> {
    let mut paths = BTreeMap::new();
    for id in read_node(tx, root)?.children {
        let node = read_node(tx, id)?;
        let key = serde_json::from_slice(&node.data)?;
        let [child] = node.children.as_slice() else { return Err("invalid projection leaf".into()); };
        unique(&mut paths, key, (id, *child))?;
    }
    Ok(paths)
}

fn read_index(tx: &Transaction, top: Node) -> Result<Index> {
    let [config, executable, annotation, dependencies, files @ ..] = top.children.as_slice() else {
        return Err("incomplete source preparation".into());
    };
    let mut entries = BTreeMap::new();
    for &id in files {
        let entry = FileEntry::from_node(id, read_node(tx, id)?)?;
        unique(&mut entries, FileKey::from(&entry.metadata.source), entry)?;
    }
    let executable_paths = read_paths(tx, *executable)?;
    let annotation_paths = read_paths(tx, *annotation)?;
    let by_projection: BTreeMap<_, _> = executable_paths.iter().map(|(key, (id, _))| (*id, key)).collect();
    let mut modules = BTreeMap::new();
    let mut module_edges = BTreeMap::new();
    for id in read_node(tx, *dependencies)?.children {
        let node = read_node(tx, id)?;
        let [local, imports @ ..] = node.children.as_slice() else {
            return Err("missing local module executable".into());
        };
        let key = *by_projection.get(local).ok_or("module outside executable paths")?;
        unique(&mut modules, key.clone(), id)?;
        module_edges.insert(key.clone(), imports.to_vec());
    }
    Ok(Index {
        config: *config, executable: *executable, annotation: *annotation,
        files: entries, modules, executable_paths, annotation_paths, module_edges,
    })
}

fn check_path(path: &PackageRelativePath) -> Result<()> {
    PackageRelativePath::new(path.as_path().to_owned())?;
    Ok(())
}

fn validate_snapshot(tx: &mut Transaction, root: NodeId, report: &mut PublishReport) -> Result<()> {
    // The newly interned root has already passed the schema boundary.
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
        admit_namespace_authority(tx, file, &config.parser.categories, report)?;
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
    Ok(())
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
    let (executable, comments) = validate_file(tx, file, configured, existing.is_none())?;
    let (fact, mut imports) = if let Some(id) = existing {
        let id = NodeId(id.as_slice().try_into()?);
        let facts = read_node(tx, id)?;
        let imports = serde_json::from_slice(&facts.data)?;
        report.namespace_reused += 1;
        (id, imports)
    } else {
        let imports = imports_from_cst(&executable.expect("requested executable tree"))?;
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
    restore_authored_spans(&mut imports, &comments);
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

fn validate_file(
    tx: &Transaction, file: &FileEntry, configured: &[String], build_executable: bool,
) -> Result<(Option<CstNode>, Vec<TypedComment>)> {
    let comments = read_comments(tx, file.annotation)?;
    let mut position = 0u32;
    let mut hasher = blake3::Hasher::new();
    let mut next_comment = comments.iter();
    let authored = read_node(tx, file.authored)?;
    if authored.kind != Kind::Syntax || syntax_kind(&authored)? != SyntaxKind::Root {
        return Err("invalid canonical Rowan root".into());
    }
    let mut builder = build_executable.then(GreenNodeBuilder::new);
    let mut stack = vec![Some((file.authored, authored, Some(file.executable)))];
    while let Some(event) = stack.pop() {
        let Some((id, node, projection)) = event else {
            builder.as_mut().expect("requested executable tree").finish_node();
            continue;
        };
        match node.kind {
            Kind::Syntax => {
                if let Some(builder) = builder.as_mut() {
                    builder.start_node(rowan::SyntaxKind(syntax_kind(&node)? as u16));
                    stack.push(None);
                }
                let projected = if projection == Some(id) {
                    node.children.clone()
                } else {
                    let projected = read_node(tx, projection.ok_or("missing CST projection")?)?;
                    if projected.kind != Kind::Syntax || projected.data != node.data {
                        return Err("incompatible executable syntax projection".into());
                    }
                    projected.children
                };
                let mut next = projected.into_iter();
                let mut pairs = Vec::with_capacity(node.children.len());
                for child in node.children {
                    let child_record = read_node(tx, child)?;
                    if child_record.kind == Kind::Token && syntax_kind(&child_record)? == SyntaxKind::TypedComment {
                        pairs.push((child, child_record, None));
                    } else {
                        pairs.push((child, child_record, Some(next.next().ok_or("incomplete executable syntax projection")?)));
                    }
                }
                if next.next().is_some() {
                    return Err("extra executable syntax nodes".into());
                }
                stack.extend(pairs.into_iter().rev().map(Some));
            }
            Kind::Token => {
                let text = token_text(&node)?;
                let end = position.checked_add(text.len().try_into()?).ok_or("CST exceeds the source range")?;
                if projection.is_none() {
                    let canonical = typed_comment_from_token(text, position, configured)
                        .ok_or("invalid canonical typed comment token")?;
                    if next_comment.next() != Some(&canonical) {
                        return Err("typed comment does not match source provenance".into());
                    }
                } else {
                    if projection != Some(id) {
                        return Err("executable token differs from authored source".into());
                    }
                    if let Some(builder) = builder.as_mut() {
                        builder.token(rowan::SyntaxKind(syntax_kind(&node)? as u16), text);
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
    Ok((builder.map(|tree| CstNode::new_root(tree.finish())), comments))
}

pub struct SourceView {
    tx: Transaction,
    root: NodeId,
    pub executable: NodeId,
    pub annotation: NodeId,
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
    let mut green = GreenNodeBuilder::new();
    let mut stack = vec![Some(root)];
    while let Some(id) = stack.pop() {
        let Some(id) = id else {
            green.finish_node();
            continue;
        };
        let node = read_node(tx, id)?;
        let kind = rowan::SyntaxKind(syntax_kind(&node)? as u16);
        match node.kind {
            Kind::Syntax => {
                green.start_node(kind);
                stack.push(None);
                stack.extend(node.children.into_iter().rev().map(Some));
            }
            Kind::Token => green.token(kind, token_text(&node)?),
            _ => return Err("invalid CST child".into()),
        }
    }
    Ok(CstNode::new_root(green.finish()))
}

impl SourceView {
    fn source_config(&self) -> Result<SourceConfig> {
        Ok(serde_json::from_slice(&read_node(&self.tx, self.config)?.data)?)
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
