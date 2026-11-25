//! Prepare base source for the incremental store. Build and Check do not use
//! this primitive yet. The resolver and parser are the ordinary compiler paths.
use rowan::{NodeOrToken, WalkEvent};
use std::collections::BTreeMap;
use std::path::Path;
use wosy_project::{ProjectConfiguration, ProjectContext, SourceGraph};
use wosy_protocol::{
    imports_from_cst, Delta, FileEntry, FileKey, FileMetadata, InstalledGitSelection, Kind,
    Namespace, NodeId, ParserPolicy, SourceConfig,
};
use wosy_syntax::{CstNode, SyntaxKind};

pub type Files = BTreeMap<FileKey, FileEntry>;

pub struct PreparedSources {
    pub root: NodeId,
    pub delta: Delta,
    pub parsed: usize,
    pub reused: usize,
}

fn pack_cst(tree: &CstNode, delta: &mut Delta) -> (NodeId, NodeId) {
    // Postorder without recursion or copying the text of every ancestor.
    let mut stack: Vec<(Vec<NodeId>, Vec<NodeId>)> = Vec::new();
    for event in tree.preorder_with_tokens() {
        match event {
            WalkEvent::Enter(NodeOrToken::Node(_)) => stack.push((Vec::new(), Vec::new())),
            WalkEvent::Enter(NodeOrToken::Token(token)) => {
                let mut data = (token.kind() as u16).to_be_bytes().to_vec();
                data.extend_from_slice(token.text().as_bytes());
                let id = delta.add(Kind::Token, data, vec![]);
                let pair = stack.last_mut().expect("token inside Rowan node");
                pair.0.push(id);
                if token.kind() != SyntaxKind::TypedComment {
                    pair.1.push(id);
                }
            }
            WalkEvent::Leave(NodeOrToken::Node(node)) => {
                let (children, exec) = stack.pop().expect("balanced Rowan tree");
                let data = (node.kind() as u16).to_be_bytes();
                let same = children == exec;
                let authored = delta.add(Kind::Syntax, data.to_vec(), children);
                let executable = if same { authored } else { delta.add(Kind::Syntax, data.to_vec(), exec) };
                if let Some(pair) = stack.last_mut() {
                    pair.0.push(authored);
                    pair.1.push(executable);
                } else {
                    return (authored, executable);
                }
            }
            WalkEvent::Leave(NodeOrToken::Token(_)) => {}
        }
    }
    unreachable!("Rowan root has an exit")
}

fn json(value: &impl serde::Serialize) -> Vec<u8> {
    serde_json::to_vec(value).expect("source record serialization")
}

fn parse_file(
    metadata: FileMetadata,
    text: String,
    categories: &[String],
    delta: &mut Delta,
) -> Result<FileEntry, String> {
    let path = metadata.source.path.as_path().to_str().ok_or("source path is not UTF-8")?;
    let source = wosy_syntax::SourceIdentity::new(
        metadata.source.project.clone(),
        metadata.source.package.clone(),
        path.to_owned(),
        metadata.revision.hex(),
    );
    let (fresh, diagnostics) = wosy_compiler::parse_source_with_snapshot(source, text, categories);
    let result = fresh.result();
    if !diagnostics.is_empty() {
        return Err(format!("{}:{}: {diagnostics:?}", result.source.package, result.source.path));
    }
    let metadata = FileMetadata {
        imports: imports_from_cst(&result.canonical_cst().root)?,
        ..metadata
    };
    let (authored, executable) = pack_cst(&result.canonical_cst().root, delta);
    let annotation = delta.add(Kind::Annotations, json(&result.comments), vec![]);
    let file = delta.add(Kind::File, json(&metadata), vec![authored, executable, annotation]);
    Ok(FileEntry { metadata, file, authored, executable, annotation })
}

/// The compiler's resolved module graph has already established reachability,
/// aliases, and cycle diagnostics. Derive module identities in dependency order,
/// reusing shared subgraphs on diamonds.
fn module_roots(graph: &SourceGraph, leaves: &[NodeId], delta: &mut Delta) -> Vec<NodeId> {
    let mut roots = vec![None; graph.nodes.len()];
    let mut stack = vec![(0, 0usize)];
    while let Some((id, next)) = stack.last_mut() {
        let edges = &graph.nodes[*id].namespace_edges;
        if let Some(edge) = edges.get(*next) {
            *next += 1;
            if roots[edge.target.0].is_none() {
                stack.push((edge.target.0, 0));
            }
        } else {
            let children = std::iter::once(leaves[*id])
                .chain(edges.iter().map(|e| roots[e.target.0].expect("dependency visited")))
                .collect();
            roots[*id] = Some(delta.add(Kind::Module, vec![], children));
            stack.pop();
        }
    }
    roots.into_iter().map(|id| id.expect("reachable module")).collect()
}

pub fn prepare(root: &Path, target: Option<&str>, base: Option<&Files>) -> Result<PreparedSources, String> {
    Ok(prepare_with_files(root, target, base)?.0)
}

pub(crate) fn prepare_with_files(
    root: &Path, target: Option<&str>, base: Option<&Files>,
) -> Result<(PreparedSources, Files), String> {
    let project = root.canonicalize().map_err(|e| e.to_string())?;
    let (cfg, manifest) = ProjectConfiguration::load_with_manifest(&project)?;
    let target = cfg.resolve_target(target)?;
    let ctx = ProjectContext::from_manifest(&project, &target, manifest.as_table())?;

    // A source preparation is not an artifact key or a completed SnapshotRoot.
    let parser = ParserPolicy {
        schema: NodeId(*wosy_syntax::base_cst_schema()),
        categories: ctx.comment_categories.clone(),
    };
    let policy = parser.id();
    let input = serde_json::to_vec(&(manifest.raw(), &cfg.resolved_packages))
        .map_err(|error| error.to_string())?;
    let inputs = NodeId(blake3::derive_key("wosy source configuration inputs v2", &input));
    let mut namespaces: BTreeMap<_, _> = cfg.resolved_packages.iter().map(|(name, p)| {
        let git = if let Some(locked) = &p.git_pin_identity {
            let patches = p.patch_stack_id
                .ok_or_else(|| format!("Git dependency {name} has no verified patch stack"))?;
            Some(InstalledGitSelection { locked: locked.clone(), patches })
        } else {
            None
        };
        Ok((name.clone(), Namespace {
            project: p.identity.project.clone(),
            source_root: Some(p.identity.path.clone()),
            git,
        }))
    }).collect::<Result<_, String>>()?;
    namespaces.insert(ctx.package.clone(), Namespace {
        project: project.to_str().ok_or("project path is not UTF-8")?.to_owned(),
        source_root: cfg.module_source_root.clone()
            .filter(|path| path.as_path() != Path::new(".")),
        git: None,
    });
    let mut delta = Delta::default();
    let (mut parsed, mut reused) = (0, 0);
    let mut files = Files::new();
    let graph = crate::project_sources::load(&project, &ctx, &cfg, |source, revision, text| {
        let key = FileKey::from(source);
        let revision = NodeId::parse_hex(&revision.0).expect("source observation is a BLAKE3 hash");
        let file = if let Some(old) = base.and_then(|files| files.get(&key)).filter(|old| {
            old.metadata.revision == revision && old.metadata.policy == policy && old.metadata.source == *source
        }) {
            reused += 1;
            old.clone()
        } else {
            parsed += 1;
            let metadata = FileMetadata {
                source: source.clone(), revision, policy, imports: Vec::new(), bytes: text.len() as u32,
            };
            parse_file(metadata, text, &ctx.comment_categories, &mut delta)?
        };
        let imports = file.metadata.imports.clone();
        files.insert(key, file);
        Ok(imports)
    })
    .map_err(|error| match error {
        crate::project_sources::Error::Observe(error) => error,
        crate::project_sources::Error::Graph(error) => format!("{error:?}"),
    })?;
    let profiles = cfg.targets[&target].profiles.keys().filter_map(|profile| {
        cfg.artifact_profile(&target, profile).ok().map(|(_, _, artifact, _)| {
            (profile.clone(), artifact.clone())
        })
    }).collect();
    let config = SourceConfig {
        entry: FileKey::from(&graph.nodes[0].source), namespaces, target, inputs, parser, profiles,
    };
    let schema = config.schema();
    let config = delta.add(Kind::Config, json(&config), vec![]);
    let mut by_location = BTreeMap::new();
    let mut leaves = Vec::new();
    for node in &graph.nodes {
        let key = FileKey::from(&node.source);
        let file = &files[&key];
        let executable = delta.add(Kind::ExecutableFile, json(&key), vec![file.executable]);
        let annotation = delta.add(Kind::AnnotationFile, json(&key), vec![file.annotation]);
        leaves.push(executable);
        by_location.insert(key, (node.id.0, executable, annotation));
    }
    let modules = module_roots(&graph, &leaves, &mut delta);
    let executable = delta.add(Kind::Executables, vec![], by_location.values().map(|v| v.1).collect());
    let annotation = delta.add(Kind::AnnotationTree, vec![], by_location.values().map(|v| v.2).collect());
    let dependencies = delta.add(Kind::Dependencies, vec![], by_location.values().map(|v| modules[v.0]).collect());
    let roots = [config, executable, annotation, dependencies].into_iter().chain(files.values().map(|f| f.file)).collect();
    let root = delta.add(Kind::SourcePreparation, schema.to_vec(), roots);
    Ok((PreparedSources { root, delta, parsed, reused }, files))
}
