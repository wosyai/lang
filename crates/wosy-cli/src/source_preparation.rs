//! Prepare base source for the incremental store. Build and Check do not use
//! this primitive yet. The resolver and parser are the ordinary compiler paths.
use crate::{artifact_inputs::{project_configuration_identity, ObservedBuilderBasis}, project_sources::{self, LiveSourceReader}};
use rowan::{NodeOrToken, WalkEvent};
use std::collections::BTreeMap;
use std::path::Path;
use wosy_project::{
    packages::{observe_comptime_watches, ComptimeWatchLimits, WatchedComptimeFile},
    ObservedClosedComptimeProfile, ProjectConfiguration, ProjectContext, SourceGraph,
};
use wosy_compiler::{
    derive_closed_comptime_fragment, derive_forwarded_pair_startup_comptime_plan,
    derive_pair_startup_call_graph_comptime_plan, ClosedComptimeFragment, ClosedComptimeRejection,
    ForwardedPairStartupComptimePlan, PairStartupCallGraphComptimePlan, ScalarTargetLayout,
};
use wosy_protocol::{
    imports_from_cst, Delta, FileEntry, FileKey, FileMetadata, InstalledGitSelection, Kind,
    Namespace, NodeId, ParserPolicy, SourceConfig,
};
use wosy_syntax::{ComptimeCst, CstNode, FreshParseResult, SourceSpan, SyntaxKind};

mod comptime_inputs;
pub use comptime_inputs::{
    ClosedComptimeInputFacts, ClosedComptimeInputSelection, ForwardedPairComptimeInputFacts,
    ForwardedPairComptimeInputSelection, PairCallGraphComptimeInputFacts, StartupComptimeInputSelection,
};

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
    singleton: bool,
    observe: impl FnOnce(&FreshParseResult) -> Result<(), String>,
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
    let imports = if singleton {
        let imports = imports_from_cst(&result.canonical_cst().root)?;
        if !imports.is_empty() { return Err("forwarded comptime input requires the selected singleton entry".into()); }
        Some(imports)
    } else { None };
    observe(&fresh)?;
    let imports = match imports {
        Some(checked) => checked,
        None => imports_from_cst(&result.canonical_cst().root)?,
    };
    let metadata = FileMetadata { imports, ..metadata };
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

/// Watches are client inputs for one exact selected authored occurrence.
/// They do not alter SourcePreparation, publish a result, or grant mounts.
#[derive(Debug, Eq, PartialEq)]
pub struct PreparedComptimeWatches {
    pub target: String,
    pub profile: String,
    pub id: String,
    pub site: SourceSpan,
    pub files: Vec<WatchedComptimeFile>,
}

struct WatchSelection<'a> {
    profile: &'a str,
    /// None selects the consumer's own ordinary singleton entry.
    owner: Option<&'a str>,
    id: &'a str,
    limits: ComptimeWatchLimits,
    site: Option<(SourceSpan, FileKey)>,
    input: Option<ComptimeInputCapture>,
}

#[derive(Clone, Copy)]
enum ComptimeInputKind { Closed, Forwarded, Graph }

impl ComptimeInputKind {
    fn name(self) -> &'static str {
        match self { Self::Closed => "closed", Self::Forwarded => "forwarded", Self::Graph => "startup graph" }
    }
}

struct ComptimeInputCapture {
    kind: ComptimeInputKind,
    layout: ScalarTargetLayout,
    ir_limit: usize,
    selected_profile: Option<ObservedClosedComptimeProfile>,
    observed: Option<ObservedComptimePlan>,
}

enum ObservedComptimePlan {
    Closed(ClosedComptimeFragment),
    Forwarded(ForwardedPairStartupComptimePlan),
    Graph(PairStartupCallGraphComptimePlan),
}

impl ObservedComptimePlan {
    fn site(&self) -> Option<&SourceSpan> {
        match self {
            Self::Closed(fragment) => Some(fragment.site()),
            Self::Forwarded(plan) => plan.calls().first().map(|call| call.facts().site()),
            Self::Graph(plan) => Some(plan.evidence().site()),
        }
    }
}

impl WatchSelection<'_> {
    fn observe(&mut self, fresh: &FreshParseResult, file: &FileKey, consumer: &str) -> Result<(), String> {
        let parsed = fresh.result();
        if parsed.source.package != self.owner.unwrap_or(consumer) { return Ok(()); }
        if let Some(input) = &mut self.input {
            let derived = match input.kind {
                ComptimeInputKind::Closed => {
                    let profile = input.selected_profile.as_ref().expect("observed selected profile");
                    derive_closed_comptime_fragment(fresh, self.id, input.layout, |name| {
                        profile.parameters().get(name).copied().map(crate::comptime_fragment::compiler_parameter)
                    }).map(ObservedComptimePlan::Closed)
                }
                ComptimeInputKind::Forwarded => derive_forwarded_pair_startup_comptime_plan(fresh, self.id, input.layout)
                    .map(ObservedComptimePlan::Forwarded),
                ComptimeInputKind::Graph => derive_pair_startup_call_graph_comptime_plan(fresh, self.id, input.layout)
                    .map(ObservedComptimePlan::Graph),
            };
            return match derived {
                Ok(plan) if self.site.is_none() => {
                    let site = plan.site().ok_or("comptime plan has no observed authored site")?;
                    self.site = Some((site.clone(), file.clone()));
                    input.observed = Some(plan);
                    Ok(())
                }
                Ok(_) => Err("comptime watch id is repeated in the reachable owning package".into()),
                Err(ClosedComptimeRejection::SiteNotFound) => Ok(()),
                Err(error) => Err(format!("{} comptime input was not prepared: {error:?}", input.kind.name())),
            };
        }
        for occurrence in parsed.root.descendants().filter_map(|node| ComptimeCst::from_node(&node)) {
            if occurrence.id.text() != self.id { continue; }
            if self.site.is_some() {
                return Err("comptime watch id is repeated in the reachable owning package".into());
            }
            self.site = Some((SourceSpan::new(parsed.source.clone(), occurrence.span), file.clone()));
        }
        Ok(())
    }
}

enum ComptimeAttachment {
    Watches(PreparedComptimeWatches),
    Closed(ClosedComptimeInputFacts),
    Forwarded(ForwardedPairComptimeInputFacts),
    Graph(PairCallGraphComptimeInputFacts),
}

/// Reuse the ordinary resolver and its actual fresh authored CST; no
/// duplicate parser, cache, evaluator launch or claim of specialization.
pub fn prepare_comptime_watches(
    root: &Path, target: &str, profile: &str, owner: &str, id: &str, limits: ComptimeWatchLimits,
) -> Result<(PreparedSources, PreparedComptimeWatches), String> {
    let selected = WatchSelection { profile, owner: Some(owner), id, limits, site: None, input: None };
    match prepare_from_observation(root, Some(target), None, None, None, Some(selected)) {
        Ok((prepared, _, _, Some(ComptimeAttachment::Watches(watches)))) => Ok((prepared, watches)),
        Ok(_) => unreachable!("successful selection produces watch observation"),
        Err(ObservationError::Source(error)) => Err(error),
        Err(ObservationError::LiveRead(_) | ObservationError::Changed) =>
            unreachable!("watch observation takes the fresh source path"),
    }
}

/// Observe one closed candidate and its selected source/configuration in the
/// same authored graph assembly. No result, tool execution, reuse key, or
/// proof that this candidate is reached during compile-time control flow.
pub fn prepare_closed_comptime_input_facts(
    root: &Path, scope: ClosedComptimeInputSelection<'_>,
) -> Result<(PreparedSources, ClosedComptimeInputFacts), String> {
    let selected = WatchSelection {
        profile: scope.profile, owner: Some(scope.owner), id: scope.id, limits: scope.watch_limits, site: None,
        input: Some(ComptimeInputCapture {
            kind: ComptimeInputKind::Closed, layout: scope.evaluator_layout, ir_limit: scope.max_ir_bytes,
            selected_profile: None, observed: None,
        }),
    };
    match prepare_from_observation(root, Some(scope.target), None, None, None, Some(selected)) {
        Ok((prepared, _, _, Some(ComptimeAttachment::Closed(facts)))) => Ok((prepared, facts)),
        Ok(_) => unreachable!("successful attachment produces closed input facts"),
        Err(ObservationError::Source(error)) => Err(error),
        Err(ObservationError::LiveRead(_) | ObservationError::Changed) =>
            unreachable!("closed comptime observation takes the fresh source path"),
    }
}

/// Capture the actual consumer's one-hop startup evidence, configured
/// evaluator declarations and owning watches during ordinary source assembly.
/// No result storage, execution or cache key is produced.
pub fn prepare_forwarded_pair_comptime_input_facts(
    root: &Path, scope: ForwardedPairComptimeInputSelection<'_>,
) -> Result<(PreparedSources, ForwardedPairComptimeInputFacts), String> {
    match prepare_startup_comptime_inputs(root, scope, ComptimeInputKind::Forwarded)? {
        (prepared, ComptimeAttachment::Forwarded(facts)) => Ok((prepared, facts)),
        _ => unreachable!("successful forwarding selection produces an input observation"),
    }
}

/// Capture the compiler's bounded startup graph with the current consumer
/// source and selected profile/watch observations. An all-bypass graph
/// still observes its own authored site and has no evaluator egress.
/// No tool execution, result storage, or public command is involved.
pub fn prepare_pair_call_graph_comptime_input_facts(
    root: &Path, scope: StartupComptimeInputSelection<'_>,
) -> Result<(PreparedSources, PairCallGraphComptimeInputFacts), String> {
    match prepare_startup_comptime_inputs(root, scope, ComptimeInputKind::Graph)? {
        (prepared, ComptimeAttachment::Graph(facts)) => Ok((prepared, facts)),
        _ => unreachable!("successful startup graph produces an input observation"),
    }
}

fn prepare_startup_comptime_inputs(
    root: &Path, scope: StartupComptimeInputSelection<'_>, kind: ComptimeInputKind,
) -> Result<(PreparedSources, ComptimeAttachment), String> {
    if !cfg!(target_os = "linux") {
        return Err(format!("{} comptime input observation requires Linux regular source and watch reads", kind.name()));
    }
    let selected = WatchSelection {
        profile: scope.profile, owner: None, id: scope.id, limits: scope.watch_limits, site: None,
        input: Some(ComptimeInputCapture {
            kind, layout: scope.evaluator_layout, ir_limit: scope.max_ir_bytes, selected_profile: None, observed: None,
        }),
    };
    match prepare_from_observation(root, Some(scope.target), None, None, None, Some(selected)) {
        Ok((prepared, _, _, Some(facts))) => Ok((prepared, facts)),
        Ok(_) => unreachable!("successful startup observation produces input facts"),
        Err(ObservationError::Source(error)) => Err(error),
        Err(ObservationError::LiveRead(_) | ObservationError::Changed) =>
            unreachable!("startup comptime observation takes the fresh source path"),
    }
}

pub(crate) fn prepare_with_files(
    root: &Path, target: Option<&str>, base: Option<&Files>,
) -> Result<(PreparedSources, Files), String> {
    match prepare_from_observation(root, target, base, None, None, None) {
        Ok((prepared, files, _, _)) => Ok((prepared, files)),
        Err(ObservationError::Source(error)) => Err(error),
        Err(ObservationError::LiveRead(_)) => unreachable!("fresh preparation uses the ordinary reader"),
        Err(ObservationError::Changed) => unreachable!("preparation accepts fresh ordinary source"),
    }
}

/// Use the ordinary locator and canonical source-root constructor. A
/// different/missing authored revision ends the probe without parsing it.
pub(crate) struct LiveSourceVerification {
    pub builder: Option<ObservedBuilderBasis>,
}

pub(crate) fn verify_live_source(
    root: &Path, target: &str, base: &Files, selected: NodeId, reader: LiveSourceReader,
    observe_builder: bool,
) -> Result<Option<LiveSourceVerification>, Box<dyn std::error::Error + Send + Sync>> {
    match prepare_from_observation(root, Some(target), Some(base), Some(reader), observe_builder.then_some(selected), None) {
        Ok((observed, _, builder, _)) if observed.root == selected => Ok(Some(LiveSourceVerification { builder })),
        Ok(_) | Err(ObservationError::Changed) => Ok(None),
        Err(ObservationError::Source(error)) => Err(error.into()),
        Err(ObservationError::LiveRead(error)) => Err(error.into()),
    }
}

enum ObservationError {
    Changed,
    Source(String),
    LiveRead(std::io::Error),
}

impl From<String> for ObservationError {
    fn from(error: String) -> Self { Self::Source(error) }
}

impl From<&str> for ObservationError {
    fn from(error: &str) -> Self { Self::Source(error.into()) }
}

fn prepare_from_observation(
    root: &Path, target: Option<&str>, base: Option<&Files>, mut live: Option<LiveSourceReader>,
    builder_source: Option<NodeId>, mut watch: Option<WatchSelection<'_>>,
) -> Result<(PreparedSources, Files, Option<ObservedBuilderBasis>, Option<ComptimeAttachment>), ObservationError> {
    let check_time = |live: &Option<LiveSourceReader>| {
        live.as_ref().map_or(Ok(()), LiveSourceReader::check_time).map_err(ObservationError::LiveRead)
    };
    check_time(&live)?;
    let project = root.canonicalize().map_err(|e| e.to_string())?;
    check_time(&live)?;
    let configuration = ProjectConfiguration::load_with_manifest(&project);
    check_time(&live)?;
    let (cfg, manifest) = configuration?;
    let target = cfg.resolve_target(target)?;
    let ctx = ProjectContext::from_manifest(&project, &target, manifest.as_table())?;
    if let Some(watch) = &mut watch {
        cfg.artifact_profile(&target, watch.profile)?;
        let owner = watch.owner.unwrap_or(&ctx.package);
        if owner != ctx.package && !cfg.resolved_packages.contains_key(owner) {
            return Err("comptime watch owner is not in the resolved project".into());
        }
        if let Some(input) = &mut watch.input {
            input.selected_profile = Some(ObservedClosedComptimeProfile::observe_input_declarations(
                &cfg, &manifest, &target, watch.profile,
            )?);
        }
    }

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
    let singleton = watch.as_ref().is_some_and(|selection| selection.owner.is_none());
    let graph = project_sources::load_with_reader(&project, &ctx, &cfg, |path, opened| match live.as_mut() {
        Some(reader) => reader.observe(path, opened).map(|id| (id, None)).map_err(ObservationError::LiveRead),
        None if singleton => {
            #[cfg(target_os = "linux")]
            {
                project_sources::read_bounded_regular_source(path, opened, wosy_compiler::CLOSED_COMPTIME_MAX_SOURCE_BYTES)
                    .map(|(id, text)| (id, Some(text))).map_err(Into::into)
            }
            #[cfg(not(target_os = "linux"))]
            {
                Err("forwarded comptime input observation requires Linux regular source reads".into())
            }
        }
        None => project_sources::read_source(path, opened).map(|(id, text)| (id, Some(text))).map_err(Into::into),
    }, |source, revision, text| -> Result<_, ObservationError> {
        let key = FileKey::from(source);
        let revision = NodeId::parse_hex(&revision.0).expect("source observation is a BLAKE3 hash");
        let file = if let Some(old) = base.and_then(|files| files.get(&key)).filter(|old| {
            old.metadata.revision == revision && old.metadata.policy == policy && old.metadata.source == *source
        }) {
            reused += 1;
            old.clone()
        } else if let Some(text) = text {
            parsed += 1;
            let metadata = FileMetadata {
                source: source.clone(), revision, policy, imports: Vec::new(), bytes: text.len() as u32,
            };
            parse_file(metadata, text, &ctx.comment_categories, &mut delta, singleton, |parsed| {
                watch.as_mut().map_or(Ok(()), |watch| watch.observe(parsed, &key, &ctx.package))
            })?
        } else {
            return Err(ObservationError::Changed);
        };
        let imports = file.metadata.imports.clone();
        files.insert(key, file);
        Ok(imports)
    });
    check_time(&live)?;
    let graph = graph.map_err(|error| match error {
        crate::project_sources::Error::Observe(error) => error,
        crate::project_sources::Error::Graph(error) => ObservationError::Source(format!("{error:?}")),
    })?;
    let watches = if let Some(watch) = watch {
        let (site, file) = watch.site.ok_or("comptime watch id does not name a reachable authored site")?;
        let owner = watch.owner.unwrap_or(&ctx.package);
        let files = observe_comptime_watches(&project, &ctx.package, &cfg, &manifest, owner, watch.id, watch.limits)?;
        Some((PreparedComptimeWatches { target: target.clone(), profile: watch.profile.into(), id: watch.id.into(), site, files }, file, watch.input))
    } else { None };
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
    let comptime = if let Some((watches, captured, input)) = watches {
        if let Some(input) = input {
            let authored = files.get(&captured).expect("compiler observed this exact file in the source assembly").authored;
            let configuration = project_configuration_identity(manifest.raw().as_bytes(), &cfg)?;
            let tools = input.selected_profile.expect("selected consumer").into_tools();
            let facts = match input.observed.expect("compiler plan for the observed authored site") {
                ObservedComptimePlan::Closed(fragment) => ComptimeAttachment::Closed(ClosedComptimeInputFacts::observe(
                    root, authored, configuration, watches, tools, fragment, input.ir_limit,
                )?),
                ObservedComptimePlan::Forwarded(plan) => ComptimeAttachment::Forwarded(ForwardedPairComptimeInputFacts::observe(
                    root, authored, configuration, watches, tools, plan, input.ir_limit,
                )?),
                ObservedComptimePlan::Graph(plan) => ComptimeAttachment::Graph(PairCallGraphComptimeInputFacts::observe(
                    root, authored, configuration, watches, tools, plan, input.ir_limit,
                )?),
            };
            Some(facts)
        } else { Some(ComptimeAttachment::Watches(watches)) }
    } else { None };
    let builder = if builder_source == Some(root) {
        Some(ObservedBuilderBasis {
            project,
            configuration_identity: project_configuration_identity(manifest.raw().as_bytes(), &cfg)?,
            configuration: cfg,
            source: project_sources::observations(graph),
        })
    } else { None };
    Ok((PreparedSources { root, delta, parsed, reused }, files, builder, comptime))
}
