//! Ordered base declarations over the admitted executable import graph.
use super::{
    child_edge, file_source, key, read_node, write_edge, write_node, BaseDeclarationsView,
    ConfiguredSourceView, QueryDisposition, Result, SourceStore, SourceView, INTERN,
};
use std::collections::HashMap;
use surrealkv::{Durability, Mode, Transaction};
use wosy_compiler::BASE_DECLARATION_ABI;
use wosy_project::SourceNodeId;
use wosy_protocol::{FileEntry, FileKey, Import, Kind, Node, NodeId};
use wosy_syntax::SourceIdentity;

const MODULE_DECLARATION_QUERIES: u8 = 14;
const MODULE_DECLARATION_ABI: &str = "wosy ordered module declarations v1";

#[derive(Clone, Copy)]
struct Projection {
    id: NodeId,
    body: NodeId,
}

#[derive(Default)]
pub struct ProjectDeclarationReport {
    pub modules_derived: usize,
    pub modules_reused: usize,
    pub bodies_derived: usize,
}

pub struct ProjectDeclarationImport {
    /// The verified declaration, with the authored range in the importing file.
    pub authored: Import,
    pub target: SourceNodeId,
}

pub struct ProjectDeclarationModule {
    pub key: FileKey,
    pub source: SourceIdentity,
    pub module: NodeId,
    pub projection: NodeId,
    pub body: NodeId,
    pub imports: Vec<ProjectDeclarationImport>,
    pub(super) file: FileEntry,
}

/// Modules are in the ordinary compiler's first-discovery initialization
/// order. The executable projection is separate from the selected author view.
pub struct ProjectDeclarationsView<'a> {
    store: &'a SourceStore,
    pub snapshot: NodeId,
    pub entry_projection: NodeId,
    pub report: ProjectDeclarationReport,
    modules: Vec<ProjectDeclarationModule>,
    _read: super::retention_plan::SourceRead,
}

impl ProjectDeclarationsView<'_> {
    pub fn modules(&self) -> &[ProjectDeclarationModule] {
        &self.modules
    }

    /// Access only a module of this admitted snapshot. Large ASTs are not
    /// decoded when listing the project; missing optional blobs are rebuildable.
    pub async fn declarations(&self, id: SourceNodeId) -> Result<Option<BaseDeclarationsView>> {
        let Some(module) = self.modules.get(id.0) else { return Ok(None); };
        let view = self.store.authored_base_declarations(&module.file, module.module, Some(module.body)).await?;
        Ok(Some(view))
    }
}

struct ModuleInput {
    id: NodeId,
    key: FileKey,
    file: FileEntry,
    imports: Vec<SourceNodeId>,
}

struct RootedOrder {
    // Ordinals assigned to the first occurrence in declared import order.
    modules: Vec<ModuleInput>,
    // Each child's projection is available before its importing module.
    postorder: Vec<usize>,
}

impl RootedOrder {
    fn from_snapshot(view: SourceView, entry: &FileKey) -> (Self, super::retention_plan::SourceRead) {
        let entry = view.modules[entry];
        let SourceView { tx, modules, mut files, mut module_edges, read, .. } = view;
        drop(tx);
        let mut locations: HashMap<_, _> = modules.into_iter().map(|(key, id)| (id, key)).collect();
        let mut order = Vec::with_capacity(files.len());
        let mut postorder = Vec::with_capacity(files.len());
        let mut visited = HashMap::with_capacity(files.len());

        enum Visit { Enter(NodeId), Exit(usize) }
        let mut pending = vec![Visit::Enter(entry)];
        while let Some(visit) = pending.pop() {
            let id = match visit {
                Visit::Enter(id) => id,
                Visit::Exit(ordinal) => {
                    postorder.push(ordinal);
                    continue;
                }
            };
            // The graph was admitted as a closed, acyclic executable DAG.
            let Some(key) = locations.remove(&id) else { continue; };
            let ordinal = order.len();
            visited.insert(id, SourceNodeId(ordinal));
            let imports = module_edges.remove(&key).expect("admitted module edges");
            let file = files.remove(&key).expect("admitted module file");
            pending.push(Visit::Exit(ordinal));
            pending.extend(imports.iter().rev().copied().map(Visit::Enter));
            order.push((id, key, file, imports));
        }
        let modules = order.into_iter().map(|(id, key, file, imports)| ModuleInput {
            id, key, file, imports: imports.into_iter().map(|id| visited[&id]).collect(),
        }).collect();
        (Self { modules, postorder }, read)
    }
}

pub(super) fn projection_key(module: NodeId) -> [u8; 33] {
    let mut hasher = blake3::Hasher::new_derive_key(MODULE_DECLARATION_ABI);
    hasher.update(BASE_DECLARATION_ABI.as_bytes())
        .update(wosy_syntax::base_cst_schema()).update(&module.0);
    key(MODULE_DECLARATION_QUERIES, NodeId(*hasher.finalize().as_bytes()))
}

fn read_projection(tx: &Transaction, module: &ModuleInput) -> Result<Option<Projection>> {
    let Some(id) = tx.get(&projection_key(module.id))? else { return Ok(None); };
    let id = NodeId(id.as_slice().try_into()?);
    let node = read_node(tx, id)?;
    let [source, body, imports @ ..] = node.children.as_slice() else {
        return Err("incomplete module declaration projection".into());
    };
    if node.kind != Kind::ModuleDeclarations || node.data != MODULE_DECLARATION_ABI.as_bytes()
        || *source != module.id
        || imports.len() != module.imports.len()
    {
        return Err("invalid module declaration projection".into());
    }
    Ok(Some(Projection { id, body: *body }))
}

impl SourceStore {
    pub async fn project_declarations(&self, root: NodeId) -> Result<Option<ProjectDeclarationsView<'_>>> {
        let Some(view) = self.snapshot(root)? else { return Ok(None); };
        self.project_declarations_for_snapshot(view.configured()?).await.map(Some)
    }

    pub(super) async fn project_declarations_for_snapshot(
        &self, snapshot: ConfiguredSourceView,
    ) -> Result<ProjectDeclarationsView<'_>> {
        let root = snapshot.source.root;
        let (RootedOrder { modules, postorder }, source_read) =
            RootedOrder::from_snapshot(snapshot.source, &snapshot.config.entry);
        let read = self.tree.begin_with_mode(Mode::ReadOnly)?;
        let cached: Vec<_> = modules.iter().map(|module| read_projection(&read, module))
            .collect::<Result<_>>()?;
        drop(read);

        let mut report = ProjectDeclarationReport::default();
        let mut bodies = Vec::new();
        for (index, (module, projection)) in modules.iter().zip(&cached).enumerate() {
            if projection.is_none() {
                let (id, disposition) = self.ensure_base_declaration_id(module.file.executable).await?;
                report.bodies_derived += usize::from(disposition == QueryDisposition::Derived);
                bodies.push((index, id));
            }
        }
        let projections = if bodies.is_empty() {
            report.modules_reused = cached.len();
            cached
        } else {
            let (projections, derived) = self.publish_projections(&modules, &postorder, &cached, &bodies).await?;
            report.modules_derived = derived;
            report.modules_reused = projections.len() - derived;
            projections
        };
        let entry_projection = projections[0].expect("entry projection").id;
        let modules = modules.into_iter().zip(projections).map(|(module, projection)| {
            let projection = projection.expect("published module projection");
            let source = file_source(&module.file)?;
            let imports = module.file.metadata.imports.iter().cloned().zip(module.imports)
                .map(|(authored, target)| ProjectDeclarationImport { authored, target })
                .collect();
            Ok(ProjectDeclarationModule {
                key: module.key, source, module: module.id, projection: projection.id,
                body: projection.body, imports, file: module.file,
            })
        }).collect::<Result<_>>()?;
        Ok(ProjectDeclarationsView {
            store: self, snapshot: root, entry_projection, report, modules, _read: source_read,
        })
    }

    async fn publish_projections(
        &self, modules: &[ModuleInput], postorder: &[usize], cached: &[Option<Projection>],
        bodies: &[(usize, NodeId)],
    ) -> Result<(Vec<Option<Projection>>, usize)> {
        // Do not hold this gate during CST extraction. Concurrent roots can
        // share AST flights; the short commit phase linearizes shared links.
        let _publication = self.project_commit.lock().await;
        let bodies: HashMap<_, _> = bodies.iter().copied().collect();
        loop {
            let mut tx = self.tree.begin()?;
            tx.set_durability(Durability::Eventual);
            let mut projections = cached.to_vec();
            let mut derived = 0;
            for &ordinal in postorder {
                if projections[ordinal].is_some() {
                    continue;
                }
                let module = &modules[ordinal];
                if let Some(existing) = read_projection(&tx, module)? {
                    projections[ordinal] = Some(existing);
                    continue;
                }
                let body = bodies[&ordinal];
                let mut children = Vec::with_capacity(2 + module.imports.len());
                children.extend([module.id, body]);
                children.extend(module.imports.iter().map(|id| projections[id.0].expect("child projected").id));
                let node = Node { kind: Kind::ModuleDeclarations, data: MODULE_DECLARATION_ABI.as_bytes().to_vec(), children };
                let bytes = node.encode();
                let id = Node::id(&bytes);
                if tx.get(&key(INTERN, id))?.is_none() {
                    write_node(&mut tx, id, &bytes)?;
                    for (index, &child) in node.children.iter().enumerate() {
                        write_edge(&mut tx, id, child_edge(node.kind, index).0, child)?;
                    }
                }
                tx.set(&projection_key(module.id), &id.0)?;
                projections[ordinal] = Some(Projection { id, body });
                derived += 1;
            }
            if derived == 0 {
                return Ok((projections, 0));
            }
            match tx.commit().await {
                Ok(()) => return Ok((projections, derived)),
                Err(surrealkv::Error::TransactionWriteConflict) => continue,
                Err(error) => return Err(error.into()),
            }
        }
    }
}
