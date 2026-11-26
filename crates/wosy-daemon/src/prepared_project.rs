//! Real compiler preparation and separately requested Build analysis.
use super::{
    base_declarations::Flights, build_receipt::{BuildInputs, Lookup}, build_session::BuildRequestRejection, read_comments,
    retention_plan::SourceLease,
    storage_worker::{QueryPhases, QueryWorker},
    BuildAnalysisOrigin, BuildAnalysisPhase, ConfiguredSourceView, ProjectDeclarationModule,
    ProjectDeclarationsView, Result, SourceStore,
};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};
use surrealkv::Mode;
use tokio::sync::Mutex as AsyncMutex;
use wosy_compiler::{
    analyze_scalar_build, prepare_scalar_project_observing_types, Diagnostic, PreparedScalarProject, ResolvedScalarProjectTypes,
    ScalarNamespaceBinding, ScalarPreparationInputs, ScalarProjectPreparationRejection,
    ScalarTargetLayout, COMPILER_IDENTITY,
};
use wosy_project::{ArtifactProfile, SourceNodeId};
use wosy_protocol::NodeId;
use wosy_syntax::{AuthoredSpanMap, SourceIdentity};

const PREPARED_PROJECT_ABI: &str = "wosy rooted typed project preparation v2";

const RESOLUTION_ENTRIES: usize = 8;
const RESOLUTION_LOGICAL_BYTES: usize = 64 << 10;

#[derive(Clone, Copy, Eq, PartialEq)]
struct ResolvedKey {
    root: NodeId,
    preparation: NodeId,
    image: NodeId,
}

type Resolutions = VecDeque<(ResolvedKey, Arc<ResolvedScalarProjectTypes>)>;

#[derive(Default)]
pub(super) struct PreparationCache {
    flights: Flights<NodeId>,
    live: Mutex<HashMap<NodeId, Weak<CanonicalPreparation>>>,
    resolutions: Mutex<Resolutions>,
}

impl PreparationCache {
    fn get(&self, key: NodeId) -> Option<Arc<CanonicalPreparation>> {
        self.live.lock().expect("typed project registry").get(&key)?.upgrade()
    }

    fn resolved(&self, key: ResolvedKey) -> Option<Arc<ResolvedScalarProjectTypes>> {
        self.resolutions.lock().expect("resolved project registry").iter()
            .find(|(stored, _)| *stored == key).map(|(_, value)| Arc::clone(value))
    }

    fn remember_resolved(
        &self, lease: &SourceLease, phases: QueryPhases<'_>, key: ResolvedKey, value: Arc<ResolvedScalarProjectTypes>,
    ) -> Result<()> {
        let mut retained = self.resolutions.lock().expect("resolved project registry");
        if retained.len() < RESOLUTION_ENTRIES { retained.reserve(1); }
        let mut pending = Some(value);
        // The existing exact preparation flight serializes equal keys. Check
        // current live session and observer at this short insertion cut.
        let retired = lease.finish_current(|| {
            phases.check_active()?;
            let prior = if retained.len() == RESOLUTION_ENTRIES { retained.pop_front() } else { None };
            retained.push_back((key, pending.take().expect("one staged handoff")));
            Ok::<_, super::Error>(prior)
        });
        drop(retained);
        drop(pending);
        drop(retired.ok_or(BuildRequestRejection::Expired)??);
        Ok(())
    }

    fn remember(&self, key: NodeId, result: &Arc<CanonicalPreparation>) {
        let mut live = self.live.lock().expect("typed project registry");
        live.retain(|_, preparation| preparation.strong_count() != 0);
        live.insert(key, Arc::downgrade(result));
    }
}

enum PreparationOutcome {
    Ready(PreparedScalarProject),
    Rejected(ScalarProjectPreparationRejection),
}

struct CanonicalPreparation {
    identity: NodeId,
    target_layout: ScalarTargetLayout,
    owners: HashMap<SourceIdentity, SourceNodeId>,
    outcome: PreparationOutcome,
    // Only a Build-analysis request initializes this projection. The weak
    // preparation registry never extends the lifetime of its diagnostics.
    build_lookup: Lookup,
    build_analysis: AsyncMutex<Option<CanonicalBuildAnalysis>>,
}

#[derive(Default)]
pub struct ProjectPreparationReport {
    pub projected_declarations: Duration,
    pub materialized_bodies: Duration,
    pub compiler_preparation: Duration,
    /// The full prepared handle remains weakly held, independent of the
    /// optional short, positively observed type-resolution handoff.
    pub reused_live_preparation: bool,
    pub reused_live_resolution: bool,
}

/// Identity of the diagnostic-only Build query for a canonical preparation.
/// Not a Check key, a Build result, or authority to use any compiled artifact.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct BuildAnalysisKey(NodeId);

impl BuildAnalysisKey {
    pub fn digest(self) -> NodeId {
        self.0
    }
}

#[derive(Clone)]
struct CanonicalBuildAnalysis {
    phase: BuildAnalysisPhase,
    diagnostics: Arc<[Diagnostic]>,
}

/// Ordinary Build analysis diagnostics, without an emitter or artifact result.
pub struct RootedBuildAnalysis {
    pub source_root: NodeId,
    pub preparation: NodeId,
    pub key: BuildAnalysisKey,
    pub phase: BuildAnalysisPhase,
    pub origin: BuildAnalysisOrigin,
    /// An optional receipt was committed with eventual cache durability.
    pub receipt_written: bool,
    pub diagnostics: Vec<Diagnostic>,
    /// Time to obtain the canonical outcome, including another caller's wait.
    pub analysis_elapsed: Duration,
    pub authored_projection_elapsed: Duration,
}

/// The owning canonical compiler preparation selected by an admitted root and
/// its real artifact profile. The authored view is independent and never
/// supplies semantic input. This is not a Build/Check result or an artifact key.
pub struct RootedPreparedProject<'a> {
    store: &'a SourceStore,
    declarations: ProjectDeclarationsView<'a>,
    canonical: Arc<CanonicalPreparation>,
    pub report: ProjectPreparationReport,
}

impl RootedPreparedProject<'_> {
    pub fn identity(&self) -> NodeId {
        self.canonical.identity
    }

    pub fn target_layout(&self) -> ScalarTargetLayout {
        self.canonical.target_layout
    }

    pub fn declarations(&self) -> &ProjectDeclarationsView<'_> {
        &self.declarations
    }

    pub fn prepared(&self) -> Option<&PreparedScalarProject> {
        match &self.canonical.outcome {
            PreparationOutcome::Ready(prepared) => Some(prepared),
            PreparationOutcome::Rejected(_) => None,
        }
    }

    pub fn authored_source(&self, owner: &SourceIdentity) -> Option<&SourceIdentity> {
        let module = self.canonical.owners.get(owner)?;
        Some(&self.declarations.modules()[module.0].source)
    }

    /// Front-end diagnostics only, reattached to this selected authored root.
    /// Do not invoke the ownership/origin analysis used by Build on rejection.
    pub fn authored_diagnostics(&self) -> Result<Vec<Diagnostic>> {
        let PreparationOutcome::Rejected(rejection) = &self.canonical.outcome else {
            return Ok(Vec::new());
        };
        project_diagnostics(self.store, &self.canonical, self.declarations.modules(), rejection.diagnostics.clone())
    }

    /// Opt in explicitly after ordinary typed preparation. Only diagnostics
    /// may be reused; neither a Build verdict nor any executable output is
    /// selected. Each caller projects its own authored revision.
    pub async fn build_analysis(&self) -> Result<RootedBuildAnalysis> {
        self.build_analysis_with(None).await
    }

    pub(super) async fn build_analysis_with(&self, worker: Option<&QueryWorker>) -> Result<RootedBuildAnalysis> {
        let phases = QueryPhases::new(self.store, worker, Some(&self.declarations.read));
        let start = Instant::now();
        let (canonical, origin, receipt_written) = {
            let mut stored = phases.lock(&self.canonical.build_analysis).await?;
            if let Some(canonical) = stored.as_ref() {
                (canonical.clone(), BuildAnalysisOrigin::Live, false)
            } else {
                let (canonical, origin, written) = self.analyze_canonical(phases).await?;
                phases.check_cutoff()?;
                *stored = Some(canonical.clone());
                (canonical, origin, written)
            }
        };
        let analysis_elapsed = start.elapsed();
        let start = Instant::now();
        let diagnostics = if canonical.diagnostics.is_empty() {
            Vec::new()
        } else {
            let preparation = Arc::clone(&self.canonical);
            let modules = Arc::clone(&self.declarations.modules);
            let canonical_diagnostics = canonical.diagnostics;
            phases.run(move |store| project_diagnostics(store, &preparation, &modules, canonical_diagnostics.to_vec())).await?
        };
        Ok(RootedBuildAnalysis {
            source_root: self.declarations.snapshot, preparation: self.identity(),
            key: BuildAnalysisKey(self.canonical.build_lookup.digest), phase: canonical.phase,
            origin, receipt_written, diagnostics, analysis_elapsed, authored_projection_elapsed: start.elapsed(),
        })
    }

    async fn analyze_canonical(
        &self, phases: QueryPhases<'_>,
    ) -> Result<(CanonicalBuildAnalysis, BuildAnalysisOrigin, bool)> {
        let phase = match self.canonical.outcome {
            PreparationOutcome::Ready(_) => BuildAnalysisPhase::Analyzed,
            PreparationOutcome::Rejected(_) => BuildAnalysisPhase::PreparationRejected,
        };
        let lookup = self.canonical.build_lookup;
        // Only this optional derivation record can fail open to ordinary
        // analysis. Source admission, preparation and projection still
        // follow their original error paths.
        if let Ok(Some(diagnostics)) = phases.run(move |store| store.read_build_receipt(&lookup, phase)).await {
            if diagnostics.iter().flat_map(|diagnostic| &diagnostic.labels)
                .all(|label| self.canonical.owners.contains_key(&label.span.source))
            {
                let analysis = CanonicalBuildAnalysis { phase, diagnostics: diagnostics.into() };
                return Ok((analysis, BuildAnalysisOrigin::Stored, false));
            }
        }
        let preparation = Arc::clone(&self.canonical);
        let diagnostics: Arc<[_]> = phases.run(move |_| Ok(match &preparation.outcome {
            PreparationOutcome::Ready(prepared) => analyze_scalar_build(prepared).diagnostics().to_vec(),
            PreparationOutcome::Rejected(rejection) => rejection.ordered_build_diagnostics(),
        }.into())).await?;
        let written = self.store.persist_build_receipt(phases, lookup, phase, Arc::clone(&diagnostics))
            .await.unwrap_or(false);
        Ok((CanonicalBuildAnalysis { phase, diagnostics }, BuildAnalysisOrigin::Derived, written))
    }
}

fn project_diagnostics(
    store: &SourceStore, preparation: &CanonicalPreparation, modules: &[ProjectDeclarationModule],
    mut diagnostics: Vec<Diagnostic>,
) -> Result<Vec<Diagnostic>> {
    if diagnostics.is_empty() { return Ok(diagnostics); }
    let mut maps = HashMap::new();
    let tx = store.tree.begin_with_mode(Mode::ReadOnly)?;
    for diagnostic in &mut diagnostics {
        for label in &mut diagnostic.labels {
            let ordinal = *preparation.owners.get(&label.span.source)
                .ok_or("compiler diagnostic outside selected project")?;
            let module = &modules[ordinal.0];
            let spans = match maps.entry(ordinal) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => entry.insert(
                    AuthoredSpanMap::from_comments(&read_comments(&tx, module.file.annotation)?)
                ),
            };
            label.span.source = module.source.clone();
            label.span.range = spans.project(label.span.range);
        }
    }
    Ok(diagnostics)
}

fn executable_owner(module: &ProjectDeclarationModule) -> SourceIdentity {
    SourceIdentity::new(
        module.source.project.clone(), module.source.package.clone(), module.source.path.clone(),
        module.file.executable.hex(),
    )
}

/// The compiler and the retention census must derive the same key from
/// a real rooted profile. This is not a result or proof/check authority.
pub(super) struct SelectedBuildProfile<Name> {
    name: Name,
    layout: ScalarTargetLayout,
}

impl<Name: AsRef<str>> SelectedBuildProfile<Name> {
    pub fn configured(name: Name, profile: &ArtifactProfile) -> Result<Self> {
        Self::try_configured(name, profile).map_err(|(_, error)| error.into())
    }

    pub(super) fn try_configured(name: Name, profile: &ArtifactProfile) -> std::result::Result<Self, (Name, String)> {
        match profile.target_layout() {
            Ok(layout) => Ok(Self {
                name,
                layout: ScalarTargetLayout { pointer_size: layout.pointer_size, pointer_alignment: layout.pointer_alignment },
            }),
            Err(error) => Err((name, error)),
        }
    }

    pub(super) fn name(&self) -> &str { self.name.as_ref() }

    pub fn identity(&self, image: Option<NodeId>, inputs: BuildInputs, projection: NodeId) -> (NodeId, Lookup) {
        let identity = preparation_identity(projection, inputs, self.name.as_ref(), self.layout);
        (identity, Lookup::new(image, inputs, projection, identity))
    }
}

fn preparation_identity(projection: NodeId, inputs: BuildInputs, profile: &str, layout: ScalarTargetLayout) -> NodeId {
    let mut hash = blake3::Hasher::new_derive_key(PREPARED_PROJECT_ABI);
    hash.update(wosy_syntax::base_cst_schema()).update(&projection.0);
    for id in inputs.ids() { hash.update(&id.0); }
    hash.update(&layout.pointer_size.to_be_bytes()).update(&layout.pointer_alignment.to_be_bytes())
        .update(COMPILER_IDENTITY.as_bytes()).update(profile.as_bytes());
    NodeId(*hash.finalize().as_bytes())
}

impl SourceStore {
    pub async fn rooted_prepared_project(
        &self, root: NodeId, profile: &str,
    ) -> Result<Option<RootedPreparedProject<'_>>> {
        let Some(snapshot) = self.snapshot(root)? else { return Ok(None); };
        self.prepare_configured(snapshot.configured()?, profile, None, None).await.map(Some)
    }

    pub(super) async fn prepare_configured(
        &self, snapshot: ConfiguredSourceView, profile: &str, worker: Option<&QueryWorker>,
        lease: Option<&SourceLease>,
    ) -> Result<RootedPreparedProject<'_>> {
        let start = Instant::now();
        let selected_profile = SelectedBuildProfile::configured(profile, snapshot.config.profiles.get(profile)
            .ok_or("artifact profile is not present in the rooted source configuration")?)?;
        let target_layout = selected_profile.layout;
        let build_inputs = BuildInputs {
            config: snapshot.source.config,
            executable: snapshot.source.executable,
            dependencies: snapshot.source.dependencies,
        };
        let declarations = self.project_declarations_for_snapshot(snapshot, worker).await?;
        let phases = QueryPhases::new(self, worker, Some(&declarations.read));
        let mut report = ProjectPreparationReport {
            projected_declarations: start.elapsed(),
            ..Default::default()
        };
        let (identity, build_lookup) = selected_profile.identity(declarations.image, build_inputs, declarations.entry_projection);

        let flight = self.prepared_projects.flights.acquire(identity);
        let _permit = phases.lock(&flight).await?;
        let canonical = if let Some(canonical) = self.prepared_projects.get(identity) {
            report.reused_live_preparation = true;
            canonical
        } else {
            let key = lease.and_then(|lease| {
                let image = declarations.image?;
                let root = declarations.snapshot;
                (lease.grant().root == root).then_some(ResolvedKey { root, preparation: identity, image })
            });
            let start = Instant::now();
            let (outcome, owners) = if let Some(cached) = key.and_then(|key| self.prepared_projects.resolved(key)) {
                let modules = Arc::clone(&declarations.modules);
                let (prepared, owners) = phases.run(move |_| {
                    let owners: Vec<_> = modules.iter().map(executable_owner).collect();
                    Ok((cached.prepare(), owners))
                }).await?;
                // A handoff contains no final inference, call or Build
                // result. Re-enter the ordinary preparation from that cut.
                report.reused_live_resolution = true;
                let outcome = prepared.map_or_else(PreparationOutcome::Rejected, PreparationOutcome::Ready);
                report.compiler_preparation = start.elapsed();
                (outcome, owners)
            } else {
                let files: Vec<_> = declarations.modules().iter()
                    .map(|module| (module.file.executable, module.body)).collect();
                let bodies = self.projected_bodies(phases, declarations.image, &files).await?;
                report.materialized_bodies = start.elapsed();
                let start = Instant::now();
                let modules = Arc::clone(&declarations.modules);
                let (outcome, owners, retained) = phases.run(move |_| {
                    let owners: Vec<_> = modules.iter().map(executable_owner).collect();
                    let mut input = Vec::with_capacity(owners.len());
                    for ((module, owner), body) in modules.iter().zip(&owners).zip(bodies) {
                        let body = body.with_executable_owner(owner.clone());
                        // Publication paired exact typed namespace
                        // occurrences in declaration order.
                        let bindings = body.namespaces().zip(&module.imports).map(|(site, link)| {
                            ScalarNamespaceBinding {
                                binding: site.binding.clone(), target: owners[link.target.0].clone(), span: site.span,
                            }
                        }).collect();
                        input.push((body, bindings));
                    }
                    let mut retained = None;
                    let outcome = match prepare_scalar_project_observing_types(
                        input, owners.clone(), ScalarPreparationInputs { target_layout },
                        |resolved| {
                            if key.is_some() { retained = resolved.copy_within(RESOLUTION_LOGICAL_BYTES).map(Arc::new); }
                        },
                    ) {
                        Ok(prepared) => PreparationOutcome::Ready(prepared),
                        Err(rejected) => {
                            retained = None;
                            PreparationOutcome::Rejected(rejected)
                        }
                    };
                    Ok((outcome, owners, retained))
                }).await?;
                report.compiler_preparation = start.elapsed();
                if let (Some(key), Some(lease), Some(retained)) = (key, lease, retained) {
                    let grant = lease.grant();
                    let _still_admitted = phases.run(move |store| Ok(store.begin_leased_source(grant, Mode::ReadOnly)?
                        .map(|(_, read)| read))).await?.ok_or(BuildRequestRejection::Expired)?;
                    self.prepared_projects.remember_resolved(lease, phases, key, retained)?;
                }
                (outcome, owners)
            };
            let owners = owners.into_iter().enumerate()
                .map(|(ordinal, identity)| (identity, SourceNodeId(ordinal))).collect();
            let canonical = Arc::new(CanonicalPreparation {
                identity, target_layout, owners, outcome,
                build_lookup,
                build_analysis: AsyncMutex::new(None),
            });
            phases.check_cutoff()?;
            self.prepared_projects.remember(identity, &canonical);
            canonical
        };
        if let Some(lease) = lease {
            lease.finish_current(|| phases.check_active()).ok_or(BuildRequestRejection::Expired)??;
        }
        Ok(RootedPreparedProject { store: self, declarations, canonical, report })
    }
}
