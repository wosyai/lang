//! Internal source-preparation store. No socket or build/check query yet.
use rowan::GreenNodeBuilder;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use surrealkv::{Durability, Mode, Transaction, Tree, TreeBuilder};
use wosy_project::{namespace_module_path, PackageRelativePath};
use wosy_protocol::{Delta, FileEntry, FileKey, Kind, Node, NodeId, SourceConfig};
use wosy_syntax::{
    typed_comment_from_token, CanonicalCstRoot, CstNode, SourceIdentity, SyntaxKind, TypedComment,
};

const NODES: u8 = 1;
const INTERN: u8 = 2;
const EDGES_OUT: u8 = 3;
const EDGES_IN: u8 = 4;
const SNAPSHOT_ROOTS: u8 = 5;

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

pub struct SourceStore(Tree);

impl SourceStore {
    pub fn open(dir: &Path, block_cache_bytes: u64) -> Result<Self> {
        let tree = TreeBuilder::new()
            .with_path(dir.to_owned())
            .with_block_cache_capacity(block_cache_bytes)
            .with_versioning(false, 0)
            .with_enable_vlog(false)
            .build()?;
        Ok(Self(tree))
    }

    pub async fn close(self) -> Result<()> {
        self.0.close().await?;
        Ok(())
    }

    /// A race restarts publication against a complete committed snapshot.
    pub async fn publish(&self, root: NodeId, delta: &Delta) -> Result<usize> {
        loop {
            let mut tx = self.0.begin()?;
            tx.set_durability(Durability::Eventual);
            if tx.get(&key(SNAPSHOT_ROOTS, root))?.is_some() {
                return Ok(0);
            }
            let (inserted, new_files) = intern_closure(&mut tx, root, delta)?;
            validate_snapshot(&tx, root, &new_files)?;
            tx.set(&key(SNAPSHOT_ROOTS, root), &root.0)?;
            match tx.commit().await {
                Ok(()) => return Ok(inserted),
                Err(surrealkv::Error::TransactionWriteConflict) => continue,
                Err(error) => return Err(error.into()),
            }
        }
    }

    pub fn snapshot(&self, root: NodeId) -> Result<Option<SourceView>> {
        let tx = self.0.begin_with_mode(Mode::ReadOnly)?;
        if tx.get(&key(SNAPSHOT_ROOTS, root))?.is_none() {
            return Ok(None);
        }
        let top = read_node(&tx, root)?;
        if !supported_top(&top) {
            return Ok(None);
        }
        let index = read_index(&tx, top)?;
        Ok(Some(SourceView {
            tx,
            executable: index.executable,
            annotation: index.annotation,
            modules: index.modules,
            files: index.files,
        }))
    }
}

fn intern_closure(
    tx: &mut Transaction, root: NodeId, delta: &Delta,
) -> Result<(usize, BTreeSet<NodeId>)> {
    let mut work = vec![(root, Expected::Exact(Kind::SourcePreparation))];
    let mut visited = HashMap::new();
    let mut inserted = 0;
    let mut new_files = BTreeSet::new();
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
        if node.kind == Kind::File {
            new_files.insert(id);
        }
        visited.insert(id, node.kind);
        tx.set(&key(NODES, id), bytes.as_slice())?;
        tx.set(&key(INTERN, id), &id.0)?;
        for (i, &child) in node.children.iter().enumerate() {
            let (role, expected) = child_edge(node.kind, i);
            tx.set(&edge(EDGES_OUT, id, role, child), &[])?;
            tx.set(&edge(EDGES_IN, child, role, id), &[])?;
            work.push((child, expected));
        }
        inserted += 1;
    }
    Ok((inserted, new_files))
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
    };
    if good { Ok(()) } else { Err("invalid base source schema or node shape".into()) }
}

fn supported_top(node: &Node) -> bool {
    node.kind == Kind::SourcePreparation
        && node.children.len() >= 5
        && node.data == wosy_syntax::base_cst_schema()
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

fn validate_snapshot(tx: &Transaction, root: NodeId, new_files: &BTreeSet<NodeId>) -> Result<()> {
    // The newly interned root has already passed the schema boundary.
    let index = read_index(tx, read_node(tx, root)?)?;
    let config: SourceConfig = serde_json::from_slice(&read_node(tx, index.config)?.data)?;
    if config.parser.schema.0 != *wosy_syntax::base_cst_schema()
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
        // Imports come from the client’s canonical namespace declarations.
        // Verify that they describe this rooted closure, not their derivation.
        let edges = index.module_edges.get(&key).ok_or("missing module for source")?;
        if source.imports.len() != edges.len() {
            return Err("module edges do not match source imports".into());
        }
        for (import, dependency) in source.imports.iter().zip(edges) {
            check_path(&import.path)?;
            if import.span.start > import.span.end || import.span.end > source.bytes {
                return Err("import outside source provenance".into());
            }
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
        if new_files.contains(&file.file) {
            validate_file(tx, file, &config.parser.categories)?;
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

fn validate_file(tx: &Transaction, file: &FileEntry, configured: &[String]) -> Result<()> {
    let comments = read_comments(tx, file.annotation)?;
    let mut position = 0u32;
    let mut hasher = blake3::Hasher::new();
    let mut next_comment = comments.iter();
    let authored = read_node(tx, file.authored)?;
    if authored.kind != Kind::Syntax || syntax_kind(&authored)? != SyntaxKind::Root {
        return Err("invalid canonical Rowan root".into());
    }
    let mut stack = vec![(file.authored, authored, Some(file.executable))];
    while let Some((id, node, projection)) = stack.pop() {
        match node.kind {
            Kind::Syntax => {
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
                stack.extend(pairs.into_iter().rev());
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
                } else if projection != Some(id) {
                    return Err("executable token differs from authored source".into());
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
    Ok(())
}

pub struct SourceView {
    tx: Transaction,
    pub executable: NodeId,
    pub annotation: NodeId,
    pub modules: BTreeMap<FileKey, NodeId>,
    pub files: BTreeMap<FileKey, FileEntry>,
}

pub struct AuthoredFile {
    pub cst: CanonicalCstRoot,
    pub comments: Vec<TypedComment>,
}

impl SourceView {
    /// Reconstitute the real Rowan tree from the selected root without reparsing.
    pub fn authored(&self, key: &FileKey) -> Result<AuthoredFile> {
        let file = self.files.get(key).ok_or("file outside selected source root")?;
        let source = &file.metadata.source;
        let path = source.path.as_path().to_str().ok_or("source path is not UTF-8")?;
        let identity = SourceIdentity::new(
            source.project.clone(), source.package.clone(), path.to_owned(), file.metadata.revision.hex(),
        );
        let mut green = GreenNodeBuilder::new();
        let mut stack = vec![Some(file.authored)];
        while let Some(id) = stack.pop() {
            let Some(id) = id else {
                green.finish_node();
                continue;
            };
            let node = read_node(&self.tx, id)?;
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
        Ok(AuthoredFile {
            cst: CanonicalCstRoot::new(identity, CstNode::new_root(green.finish())),
            comments: read_comments(&self.tx, file.annotation)?,
        })
    }
}
