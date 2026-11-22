use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs;
use wosy_compiler::{derive_scalar_declarations_from_cst, parse_source, ScalarNamespaceBinding};
use wosy_project::{
    load_reachable_source_graph, ContentRevision, NamespaceEdgeInput, PackageRelativePath,
    ProjectConfiguration, ProjectContext, SourceIdentity as ProjectSourceIdentity, SourceNodeInput,
    SourceSpan as ProjectSourceSpan,
};
use wosy_syntax::SourceIdentity;

type PersistCst = fn(&std::path::Path, &wosy_compiler::ParseOutput) -> Result<(), String>;

pub(crate) struct CanonicalProjectInputs {
    pub(crate) declarations: Vec<(wosy_compiler::ScalarSourceDeclarations, Vec<ScalarNamespaceBinding>)>,
    pub(crate) initialization_order: Vec<SourceIdentity>,
    pub(crate) graph: wosy_project::SourceGraph,
    pub(crate) texts: BTreeMap<ProjectSourceIdentity, String>,
    #[cfg(feature = "internal-checker")]
    pub(crate) proofs: BTreeMap<PackageRelativePath, (wosy_compiler::ScalarProofDeclarations, Vec<ScalarNamespaceBinding>)>,
}

pub(crate) enum CanonicalLoadPurpose {
    Build(PersistCst),
    #[cfg(feature = "internal-checker")]
    Check,
    #[cfg(feature = "internal-checker")]
    Proof(HashSet<PackageRelativePath>),
    #[cfg(feature = "internal-checker")]
    #[allow(dead_code)]
    LocalProofs,
}

enum CanonicalSourceDeclarations {
    Ordinary(wosy_compiler::ScalarSourceDeclarations),
    #[cfg(feature = "internal-checker")]
    Proof(wosy_compiler::ScalarProofDeclarations),
}

impl CanonicalSourceDeclarations {
    fn declarations(&self) -> &wosy_compiler::ScalarSourceDeclarations {
        match self {
            Self::Ordinary(declarations) => declarations,
            #[cfg(feature = "internal-checker")]
            Self::Proof(proof) => proof.declarations(),
        }
    }
}

#[derive(Debug)]
pub(crate) enum CanonicalLoadFailure {
    Input(String),
    Syntax {
        source: SourceIdentity,
        text: String,
        diagnostics: Vec<wosy_compiler::Diagnostic>,
    },
    Derivation {
        diagnostics: Vec<wosy_compiler::Diagnostic>,
        texts: BTreeMap<ProjectSourceIdentity, String>,
    },
    Graph {
        error: wosy_project::GraphLoadError,
        texts: BTreeMap<ProjectSourceIdentity, String>,
    },
}

impl From<String> for CanonicalLoadFailure {
    fn from(error: String) -> Self {
        Self::Input(error)
    }
}

fn canonical_syntax_failure(output: wosy_compiler::ParseOutput) -> CanonicalLoadFailure {
    CanonicalLoadFailure::Syntax {
        source: output.result.source,
        text: output.result.text,
        diagnostics: output.diagnostics,
    }
}

pub(crate) fn canonical_source_identity(
    project_identity: String,
    package: &str,
    path: &PackageRelativePath,
    text: &str,
) -> SourceIdentity {
    SourceIdentity::new(
        project_identity,
        package.to_owned(),
        path.as_path().to_string_lossy().replace('\\', "/"),
        blake3::hash(text.as_bytes()).to_hex().to_string(),
    )
}

pub(crate) fn parse_canonical_ordinary_source(
    source: SourceIdentity,
    text: String,
    comment_categories: &[String],
    persist_in: Option<(&std::path::Path, PersistCst)>,
) -> Result<(String, wosy_compiler::ScalarSourceDeclarations), CanonicalLoadFailure> {
    let output = parse_source(source, text, comment_categories);
    if !output.diagnostics.is_empty() {
        return Err(canonical_syntax_failure(output));
    }
    if let Some((root, persist_cst)) = persist_in {
        persist_cst(root, &output)?;
    }
    let declarations = derive_scalar_declarations_from_cst(output.result.canonical_cst());
    Ok((output.result.text, declarations))
}

#[cfg(feature = "internal-checker")]
pub(crate) struct DormantSourceBoundaries<'a> {
    root: &'a std::path::Path,
    project: &'a ProjectContext,
    dependency_prefixes: Vec<&'a std::path::Path>,
}

#[cfg(feature = "internal-checker")]
impl<'a> DormantSourceBoundaries<'a> {
    pub(crate) fn new(
        root: &'a std::path::Path, project: &'a ProjectContext, configuration: &'a ProjectConfiguration,
    ) -> Self {
        let dependency_prefixes = configuration.resolved_packages.values()
            .filter_map(|dependency| dependency.project_root.strip_prefix(root).ok())
            .collect();
        Self { root, project, dependency_prefixes }
    }

    pub(crate) fn validate(&self, identity: &ProjectSourceIdentity) -> Result<(), CanonicalLoadFailure> {
        use std::path::Component;
        let relative = identity.path.as_path();
        let spelling = relative.to_str().ok_or_else(|| "dormant source path must be UTF-8".to_owned())?;
        if spelling.is_empty() || spelling.contains('\\')
            || !relative.components().all(|component| matches!(component, Component::Normal(_)))
            || relative.components().collect::<std::path::PathBuf>().as_os_str() != relative.as_os_str()
        {
            return Err(format!("dormant source path is not canonical: {spelling}").into());
        }
        if identity.project != self.root.to_string_lossy() || identity.package != self.project.package
            || relative.starts_with("deps")
            || self.dependency_prefixes.iter().any(|prefix| relative.starts_with(prefix))
        {
            return Err(format!("dormant source is outside the current package: {spelling}").into());
        }
        Ok(())
    }
}

#[cfg(feature = "internal-checker")]
#[allow(dead_code)]
pub(crate) fn load_explicit_dormant_proof_sources(
    root: &std::path::Path, project: &ProjectContext, configuration: &ProjectConfiguration,
    paths: Vec<PackageRelativePath>, texts: &BTreeMap<ProjectSourceIdentity, String>,
) -> Result<Vec<wosy_compiler::ScalarProofDeclarations>, CanonicalLoadFailure> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let boundaries = DormantSourceBoundaries::new(root, project, configuration);
    let identities = paths.into_iter().map(|path| {
        ProjectSourceIdentity::new(root.to_string_lossy().into_owned(), project.package.clone(), path)
    }).collect::<Vec<_>>();
    let mut selected = HashSet::with_capacity(identities.len());
    for identity in &identities {
        boundaries.validate(identity)?;
        if texts.contains_key(identity) || !selected.insert(identity) {
            return Err(format!("dormant source was already selected: {}", identity.path.as_path().display()).into());
        }
    }
    identities.into_iter().map(|identity| {
        let path = root.join(identity.path.as_path());
        let resolved = path.canonicalize().map_err(|error|
            format!("failed to resolve dormant source {}: {error}", identity.path.as_path().display())
        )?;
        if resolved != path || !resolved.is_file() {
            return Err(format!("dormant source must be a regular non-aliased file: {}", identity.path.as_path().display()).into());
        }
        let text = fs::read_to_string(&resolved).map_err(|error|
            format!("failed to read dormant source {}: {error}", identity.path.as_path().display())
        )?;
        let source = canonical_source_identity(identity.project, &identity.package, &identity.path, &text);
        wosy_compiler::parse_proof_source(source, text, &project.comment_categories)
            .map(|(_, proof)| proof).map_err(canonical_syntax_failure)
    }).collect()
}

pub(crate) fn load_canonical_project_inputs(
    root: &std::path::Path,
    project: &ProjectContext,
    configuration: &ProjectConfiguration,
    purpose: CanonicalLoadPurpose,
    mut on_extraction: impl FnMut(),
) -> Result<CanonicalProjectInputs, CanonicalLoadFailure> {
    let root_path = PackageRelativePath::new(project.source_root.clone())?;
    let root_project = root.to_string_lossy().into_owned();
    let mut discovered = HashMap::from([(
        project.package.as_str(),
        HashSet::from([root_path.clone()]),
    )]);
    let mut pending = VecDeque::from([(project.package.clone(), root_path)]);
    let mut nodes = Vec::new();
    let mut declarations_by_path = BTreeMap::new();
    let mut texts = BTreeMap::new();

    while let Some((package, path)) = pending.pop_front() {
        let (package_root, project_identity) = if package == project.package {
            (root.to_path_buf(), root_project.clone())
        } else {
            let resolved = configuration
                .resolved_packages
                .get(&package)
                .ok_or_else(|| format!("dependency package {package} is missing"))?;
            (
                resolved.project_root.clone(),
                resolved.identity.project.clone(),
            )
        };
        let source_path = package_root.join(path.as_path());
        let text = fs::read_to_string(&source_path)
            .map_err(|error| format!("failed to read {}: {error}", source_path.display()))?;
        let source = canonical_source_identity(project_identity, &package, &path, &text);
        #[cfg(feature = "internal-checker")]
        let parse_proofs = match &purpose {
            CanonicalLoadPurpose::LocalProofs => package == project.package,
            CanonicalLoadPurpose::Proof(selected) => package == project.package && selected.contains(&path),
            _ => false,
        };
        let (text, declarations) = match &purpose {
            #[cfg(feature = "internal-checker")]
            _ if parse_proofs => {
                let (text, proof) = wosy_compiler::parse_proof_source(source, text, &project.comment_categories)
                    .map_err(canonical_syntax_failure)?;
                (text, CanonicalSourceDeclarations::Proof(proof))
            }
            _ => {
                let (text, declarations) = parse_canonical_ordinary_source(
                    source, text, &project.comment_categories,
                    match &purpose {
                        CanonicalLoadPurpose::Build(persist) => Some((root, *persist)),
                        #[cfg(feature = "internal-checker")]
                        CanonicalLoadPurpose::Check | CanonicalLoadPurpose::Proof(_) | CanonicalLoadPurpose::LocalProofs => None,
                    },
                )?;
                (text, CanonicalSourceDeclarations::Ordinary(declarations))
            }
        };
        on_extraction();
        let source = declarations.declarations().source();
        let mut namespace_edges = Vec::new();
        for namespace in declarations.declarations().namespaces() {
            let module_path = serde_json::from_str::<String>(&namespace.path)
                .map_err(|error| format!("invalid namespace path: {error}"))?;
            let module_path = PackageRelativePath::new(module_path.into())?;
            let (target_package, target_root, target_path) = if namespace.package == project.package {
                (project.package.as_str(), root, Cow::Borrowed(&module_path))
            } else {
                let (package, resolved) = configuration
                    .resolved_packages
                    .get_key_value(&namespace.package)
                    .ok_or_else(|| format!("dependency package {} is missing", namespace.package))?;
                let target_path = PackageRelativePath::new(
                    resolved.identity.path.as_path().join(module_path.as_path()),
                )?;
                (
                    package.as_str(),
                    resolved.project_root.as_path(),
                    Cow::Owned(target_path),
                )
            };
            let already_discovered = discovered
                .get(target_package)
                .is_some_and(|paths| paths.contains(target_path.as_ref()));
            if !already_discovered && target_root.join(target_path.as_path()).is_file() {
                let target_path = target_path.into_owned();
                discovered
                    .entry(target_package)
                    .or_default()
                    .insert(target_path.clone());
                pending.push_back((target_package.to_owned(), target_path));
            }
            namespace_edges.push(NamespaceEdgeInput {
                origin: namespace.package.clone(),
                path: module_path,
                binding: namespace.binding.clone(),
                span: ProjectSourceSpan {
                    source: ProjectSourceIdentity::new(
                        source.project.clone(),
                        source.package.clone(),
                        path.clone(),
                    ),
                    range: wosy_project::ByteSpan::new(namespace.span.start, namespace.span.end),
                },
            });
        }
        let graph_source = ProjectSourceIdentity::new(
            source.project.clone(),
            source.package.clone(),
            path.clone(),
        );
        nodes.push(SourceNodeInput {
            source: graph_source.clone(),
            content_revision: ContentRevision(source.revision.clone()),
            namespace_edges,
        });
        declarations_by_path.insert((package, path), declarations);
        texts.insert(graph_source, text);
    }
    drop((discovered, pending));

    let root_node = nodes
        .first()
        .cloned()
        .ok_or_else(|| "project root source is missing".to_owned())?;
    let graph = match load_reachable_source_graph(
        &project.package,
        root_node,
        nodes,
        &configuration.resolved_packages,
    ) {
        Ok(graph) => graph,
        Err(error) => return Err(CanonicalLoadFailure::Graph { error, texts }),
    };
    let mut project_declarations = Vec::new();
    #[cfg(feature = "internal-checker")]
    let mut proofs = BTreeMap::new();
    let mut source_by_path = BTreeMap::new();
    let mut derivation_diagnostics = Vec::new();
    for node in &graph.nodes {
        let source = compiler_source_identity(&node.source, &node.content_revision);
        source_by_path.insert(
            (node.source.package.clone(), node.source.path.clone()),
            source,
        );
    }
    for node in &graph.nodes {
        let declarations = declarations_by_path
            .remove(&(node.source.package.clone(), node.source.path.clone()))
            .ok_or_else(|| {
                format!(
                    "missing canonical CST for {}",
                    node.source.path.as_path().display()
                )
            })?;
        derivation_diagnostics.extend_from_slice(declarations.declarations().diagnostics());
        let namespace_bindings = node
            .namespace_edges
            .iter()
            .map(|edge| {
                Ok(ScalarNamespaceBinding {
                    binding: edge.binding.clone(),
                    target: source_by_path
                        .get(&(edge.origin.clone(), edge.path.clone()))
                        .cloned()
                        .ok_or_else(|| {
                            format!("missing module {}", edge.path.as_path().display())
                        })?,
                    span: wosy_syntax::ByteSpan::new(edge.span.range.start, edge.span.range.end),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        match declarations {
            CanonicalSourceDeclarations::Ordinary(declarations) => project_declarations.push((declarations, namespace_bindings)),
            #[cfg(feature = "internal-checker")]
            CanonicalSourceDeclarations::Proof(declarations) => {
                proofs.insert(node.source.path.clone(), (declarations, namespace_bindings));
            }
        }
    }
    if !derivation_diagnostics.is_empty() {
        return Err(CanonicalLoadFailure::Derivation { diagnostics: derivation_diagnostics, texts });
    }
    let initialization_order = graph
        .initialization_order
        .iter()
        .map(|id| {
            let node = graph
                .nodes
                .get(id.0)
                .ok_or_else(|| format!("missing initialization node {}", id.0))?;
            source_by_path
                .get(&(node.source.package.clone(), node.source.path.clone()))
                .cloned()
                .ok_or_else(|| format!("missing module {}", node.source.path.as_path().display()))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(CanonicalProjectInputs {
        declarations: project_declarations,
        initialization_order,
        graph,
        texts,
        #[cfg(feature = "internal-checker")]
        proofs,
    })
}

pub(crate) fn compiler_source_identity(
    source: &ProjectSourceIdentity,
    revision: &ContentRevision,
) -> SourceIdentity {
    SourceIdentity::new(
        source.project.clone(),
        source.package.clone(),
        source.path.as_path().to_string_lossy().replace('\\', "/"),
        revision.0.clone(),
    )
}
