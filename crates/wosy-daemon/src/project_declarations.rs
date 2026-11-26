//! Ordered base declarations over the admitted executable import graph.
use super::{
    base_declarations::authored_declaration_diagnostics, child_edge, file_source, key, read_node,
    read_node_with, retention_plan::SourceRead, source_proposal::CstWork,
    storage_worker::{QueryPhases, QueryWorker}, write_edge, write_node, BaseDeclarationsView,
    ConfiguredSourceView, Index, QueryDisposition, Result, SourceStore, SourceView, INTERN,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use surrealkv::{Durability, Mode, Transaction};
use wosy_compiler::{Diagnostic, ScalarSourceDeclarations, BASE_DECLARATION_ABI};
use wosy_project::SourceNodeId;
use wosy_protocol::{FileEntry, FileKey, Import, Kind, Node, NodeId};
use wosy_syntax::{AuthoredSpanMap, ByteSpan, SourceIdentity, TypedComment};

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

/// One BASE declaration body from an admitted, not-yet-published proposal.
pub struct PreparedBaseModule {
    module: ProjectDeclarationModule,
    declarations: Arc<ScalarSourceDeclarations>,
    spans: AuthoredSpanMap,
}

impl PreparedBaseModule {
    pub fn module(&self) -> &ProjectDeclarationModule { &self.module }

    pub fn executable_declarations(&self) -> &ScalarSourceDeclarations { &self.declarations }

    pub fn authored_span(&self, span: ByteSpan) -> ByteSpan { self.spans.project(span) }

    pub fn authored_diagnostics(&self) -> Vec<Diagnostic> {
        authored_declaration_diagnostics(&self.declarations, &self.module.source, &self.spans)
    }
}

pub(super) struct UnpublishedDeclarations {
    pub entry_projection: NodeId,
    pub modules: Vec<PreparedBaseModule>,
    pub unique_bodies: usize,
    pub declaration_bytes: usize,
}

/// Modules are in the ordinary compiler's first-discovery initialization
/// order. The executable projection is separate from the selected author view.
pub struct ProjectDeclarationsView<'a> {
    store: &'a SourceStore,
    pub snapshot: NodeId,
    pub entry_projection: NodeId,
    pub report: ProjectDeclarationReport,
    pub(super) modules: Arc<[ProjectDeclarationModule]>,
    pub(super) image: Option<NodeId>,
    pub(super) read: SourceRead,
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
    fn from_snapshot(view: SourceView, entry: &FileKey) -> (Self, SourceRead) {
        let SourceView { tx, modules, files, module_edges, read, .. } = view;
        drop(tx);
        (Self::from_parts(entry, modules, files, module_edges), read)
    }

    fn from_parts(
        entry: &FileKey, modules: BTreeMap<FileKey, NodeId>, mut files: BTreeMap<FileKey, FileEntry>,
        mut module_edges: BTreeMap<FileKey, Vec<NodeId>>,
    ) -> Self {
        let entry = modules[entry];
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
        Self { modules, postorder }
    }
}

pub(super) fn projection_key(image: NodeId, module: NodeId) -> [u8; 33] {
    let mut hasher = blake3::Hasher::new_derive_key(MODULE_DECLARATION_ABI);
    hasher.update(BASE_DECLARATION_ABI.as_bytes())
        .update(wosy_syntax::base_cst_schema()).update(&image.0).update(&module.0);
    key(MODULE_DECLARATION_QUERIES, NodeId(*hasher.finalize().as_bytes()))
}

fn read_projection(tx: &Transaction, image: NodeId, module: &ModuleInput) -> Result<Option<Projection>> {
    let Some(id) = tx.get(&projection_key(image, module.id))? else { return Ok(None); };
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

fn compose_projection(
    module: &ModuleInput, body: NodeId, projections: &[Option<Projection>],
) -> (Projection, Node, Vec<u8>) {
    let mut children = Vec::with_capacity(2 + module.imports.len());
    children.extend([module.id, body]);
    children.extend(module.imports.iter().map(|id| projections[id.0].expect("child projected").id));
    let node = Node { kind: Kind::ModuleDeclarations, data: MODULE_DECLARATION_ABI.as_bytes().to_vec(), children };
    let bytes = node.encode();
    (Projection { id: Node::id(&bytes), body }, node, bytes)
}

struct ProjectionPlan {
    order: RootedOrder,
    cached: Vec<Option<Projection>>,
    bodies: HashMap<usize, NodeId>,
}

impl ProjectionPlan {
    fn in_memory(&self) -> Vec<Option<Projection>> {
        compose_memory(&self.order, self.cached.clone(), |index| self.bodies[&index])
    }
}

fn compose_memory(
    order: &RootedOrder, mut projections: Vec<Option<Projection>>, body_id: impl Fn(usize) -> NodeId,
) -> Vec<Option<Projection>> {
    for &index in &order.postorder {
        if projections[index].is_none() {
            projections[index] = Some(compose_projection(&order.modules[index], body_id(index), &projections).0);
        }
    }
    projections
}

fn projected_module(module: ModuleInput, projection: Projection) -> Result<ProjectDeclarationModule> {
    let source = file_source(&module.file)?;
    let imports = module.file.metadata.imports.iter().cloned().zip(module.imports)
        .map(|(authored, target)| ProjectDeclarationImport { authored, target })
        .collect();
    Ok(ProjectDeclarationModule {
        key: module.key, source, module: module.id, projection: projection.id,
        body: projection.body, imports, file: module.file,
    })
}

impl SourceStore {
    pub(super) fn prepare_unpublished_declarations(
        &self, tx: &Transaction, entry: &FileKey, index: Index, work: &mut CstWork, body_bytes: usize,
    ) -> Result<UnpublishedDeclarations> {
        let order = RootedOrder::from_parts(entry, index.modules, index.files, index.module_edges);
        let mut remaining = body_bytes;
        let mut bodies = HashMap::new();
        for module in &order.modules {
            let executable = module.file.executable;
            if let std::collections::hash_map::Entry::Vacant(body) = bodies.entry(executable) {
                let (id, declarations) = self.prepare_base_body(tx, executable, work, &mut remaining)?;
                body.insert((id, Arc::new(declarations)));
            }
        }
        let projections = compose_memory(&order, vec![None; order.modules.len()], |i| {
            bodies[&order.modules[i].file.executable].0
        });
        let entry_projection = projections[0].expect("entry projection").id;
        let modules = order.modules.into_iter().zip(projections).map(|(input, projection)| {
            let annotations = read_node_with(tx, input.file.annotation, |bytes| work.read(bytes))?;
            let comments: Vec<TypedComment> = serde_json::from_slice(&annotations.data)?;
            let declarations = Arc::clone(&bodies[&input.file.executable].1);
            let module = projected_module(input, projection.expect("prepared module projection"))?;
            Ok(PreparedBaseModule { module, declarations, spans: AuthoredSpanMap::from_comments(&comments) })
        }).collect::<Result<Vec<_>>>()?;
        Ok(UnpublishedDeclarations {
            entry_projection, modules, unique_bodies: bodies.len(), declaration_bytes: body_bytes - remaining,
        })
    }

    pub async fn project_declarations(&self, root: NodeId) -> Result<Option<ProjectDeclarationsView<'_>>> {
        let Some(view) = self.snapshot(root)? else { return Ok(None); };
        self.project_declarations_for_snapshot(view.configured()?, None).await.map(Some)
    }

    pub(super) async fn project_declarations_for_snapshot(
        &self, snapshot: ConfiguredSourceView, worker: Option<&QueryWorker>,
    ) -> Result<ProjectDeclarationsView<'_>> {
        // The configured input carries the root guard while the initial
        // project-wide index read takes place on the blocking executor.
        let (root, order, read, image, cached) = QueryPhases::new(self, worker, None).run(move |store| {
            let root = snapshot.source.root;
            let (order, source_read) = RootedOrder::from_snapshot(snapshot.source, &snapshot.config.entry);
            let image = store.derivation_image();
            let cached = if let Some(image) = image {
                let read = store.tree.begin_with_mode(Mode::ReadOnly)?;
                order.modules.iter().map(|module| read_projection(&read, image, module)).collect::<Result<Vec<_>>>()?
            } else {
                vec![None; order.modules.len()]
            };
            Ok((root, order, source_read, image, cached))
        }).await?;
        let phases = QueryPhases::new(self, worker, Some(&read));

        let missing: BTreeSet<_> = order.modules.iter().zip(&cached)
            .filter_map(|(module, found)| found.is_none().then_some(module.file.executable)).collect();
        let inputs: Vec<_> = missing.into_iter().collect();
        let mut report = ProjectDeclarationReport::default();
        let (order, projections) = if inputs.is_empty() {
            report.modules_reused = cached.len();
            (order, cached)
        } else {
            let output = self.ensure_base_ids(phases, image, &inputs).await?;
            let bodies: HashMap<_, _> = inputs.into_iter().zip(output).map(|(executable, (id, disposition))| {
                report.bodies_derived += usize::from(disposition == QueryDisposition::Derived);
                (executable, id)
            }).collect();
            let bodies = order.modules.iter().zip(&cached).enumerate().filter_map(|(index, (module, hit))| {
                hit.is_none().then(|| (index, bodies[&module.file.executable]))
            }).collect();
            let plan = Arc::new(ProjectionPlan { order, cached, bodies });
            let projections = if let Some(image) = image {
                let (projections, derived) = self.publish_projections(phases, image, &plan).await?;
                report.modules_derived = derived;
                report.modules_reused = projections.len() - derived;
                projections
            } else {
                let memory = Arc::clone(&plan);
                let projections = phases.run(move |_| Ok(memory.in_memory())).await?;
                report.modules_derived = projections.len();
                projections
            };
            let owned = Arc::try_unwrap(plan).ok().expect("projection phases completed");
            (owned.order, projections)
        };
        let entry_projection = projections[0].expect("entry projection").id;
        let modules = order.modules.into_iter().zip(projections)
            .map(|(module, projection)| projected_module(module, projection.expect("published module projection")))
            .collect::<Result<Arc<[_]>>>()?;
        Ok(ProjectDeclarationsView { store: self, snapshot: root, entry_projection, report, modules, image, read })
    }

    async fn publish_projections(
        &self, phases: QueryPhases<'_>, image: NodeId, plan: &Arc<ProjectionPlan>,
    ) -> Result<(Vec<Option<Projection>>, usize)> {
        // Do not hold this gate during CST extraction. Concurrent roots can
        // share AST flights; the short commit phase linearizes shared links.
        let _publication = self.project_commit.lock().await;
        loop {
            let step = Arc::clone(plan);
            let (tx, projections, derived) = phases.run(move |store|
                store.projection_transaction(image, &step)
            ).await?;
            let Some(mut tx) = tx else { return Ok((projections, derived)); };
            match tx.commit().await {
                Ok(()) => return Ok((projections, derived)),
                Err(surrealkv::Error::TransactionWriteConflict) => continue,
                Err(error) => return Err(error.into()),
            }
        }
    }

    fn projection_transaction(
        &self, image: NodeId, plan: &ProjectionPlan,
    ) -> Result<(Option<Transaction>, Vec<Option<Projection>>, usize)> {
        let mut tx = self.tree.begin()?;
        tx.set_durability(Durability::Eventual);
        let mut projections = plan.cached.clone();
        let mut derived = 0;
        for &ordinal in &plan.order.postorder {
            if projections[ordinal].is_some() { continue; }
            let module = &plan.order.modules[ordinal];
            if let Some(existing) = read_projection(&tx, image, module)? {
                projections[ordinal] = Some(existing);
                continue;
            }
            let body = plan.bodies[&ordinal];
            let (projection, node, bytes) = compose_projection(module, body, &projections);
            if tx.get(&key(INTERN, projection.id))?.is_none() {
                write_node(&mut tx, projection.id, &bytes)?;
                for (index, &child) in node.children.iter().enumerate() {
                    write_edge(&mut tx, projection.id, child_edge(node.kind, index).0, child)?;
                }
            }
            tx.set(&projection_key(image, module.id), &projection.id.0)?;
            projections[ordinal] = Some(projection);
            derived += 1;
        }
        Ok(((derived != 0).then_some(tx), projections, derived))
    }
}
