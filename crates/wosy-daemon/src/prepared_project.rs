//! Real compiler preparation and separately requested Build analysis.
use super::{
    base_declarations::Flights, build_receipt::{BuildInputs, Lookup}, read_comments,
    ProjectDeclarationModule, ProjectDeclarationsView,
    Result, SourceStore,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};
use surrealkv::Mode;
use tokio::sync::OnceCell;
use wosy_compiler::{
    analyze_scalar_build, prepare_scalar_project, Diagnostic, PreparedScalarProject,
    ScalarNamespaceBinding, ScalarPreparationInputs, ScalarProjectPreparationRejection,
    ScalarTargetLayout, COMPILER_IDENTITY,
};
use wosy_project::SourceNodeId;
use wosy_protocol::NodeId;
use wosy_syntax::{AuthoredSpanMap, SourceIdentity};

const PREPARED_PROJECT_ABI: &str = "wosy rooted typed project preparation v2";

#[derive(Default)]
pub(super) struct PreparationCache {
    flights: Flights<NodeId>,
    live: Mutex<HashMap<NodeId, Weak<CanonicalPreparation>>>,
}

impl PreparationCache {
    fn get(&self, key: NodeId) -> Option<Arc<CanonicalPreparation>> {
        self.live.lock().expect("typed project registry").get(&key)?.upgrade()
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
    build_analysis: OnceCell<CanonicalBuildAnalysis>,
}

#[derive(Default)]
pub struct ProjectPreparationReport {
    pub projected_declarations: Duration,
    pub materialized_bodies: Duration,
    pub compiler_preparation: Duration,
    /// No typed state survives after all users of the exact handle drop it.
    pub reused_live_preparation: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum BuildAnalysisPhase {
    PreparationRejected,
    Analyzed,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BuildAnalysisOrigin {
    Derived,
    Live,
    Stored,
}

struct CanonicalBuildAnalysis {
    phase: BuildAnalysisPhase,
    diagnostics: Vec<Diagnostic>,
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
        self.project_diagnostics(rejection.diagnostics.clone())
    }

    /// Opt in explicitly after ordinary typed preparation. Only diagnostics
    /// may be reused; neither a Build verdict nor any executable output is
    /// selected. Each caller projects its own authored revision.
    pub async fn build_analysis(&self) -> Result<RootedBuildAnalysis> {
        let start = Instant::now();
        let mut origin = BuildAnalysisOrigin::Live;
        let mut receipt_written = false;
        let canonical = self.canonical.build_analysis.get_or_init(|| async {
            let phase = match self.canonical.outcome {
                PreparationOutcome::Ready(_) => BuildAnalysisPhase::Analyzed,
                PreparationOutcome::Rejected(_) => BuildAnalysisPhase::PreparationRejected,
            };
            let lookup = &self.canonical.build_lookup;
            // Only this optional derivation record can fail open to ordinary
            // analysis. Source admission, preparation and projection still
            // follow their original error paths.
            if let Ok(Some(diagnostics)) = self.store.read_build_receipt(lookup, phase) {
                if diagnostics.iter().flat_map(|diagnostic| &diagnostic.labels)
                    .all(|label| self.canonical.owners.contains_key(&label.span.source))
                {
                    origin = BuildAnalysisOrigin::Stored;
                    return CanonicalBuildAnalysis { phase, diagnostics };
                }
            }
            origin = BuildAnalysisOrigin::Derived;
            let diagnostics = match &self.canonical.outcome {
                PreparationOutcome::Ready(prepared) => analyze_scalar_build(prepared).diagnostics().to_vec(),
                PreparationOutcome::Rejected(rejection) => rejection.ordered_build_diagnostics(),
            };
            receipt_written = self.store.persist_build_receipt(lookup, phase, &diagnostics).await.unwrap_or(false);
            CanonicalBuildAnalysis { phase, diagnostics }
        }).await;
        let analysis_elapsed = start.elapsed();
        let start = Instant::now();
        let diagnostics = self.project_diagnostics(canonical.diagnostics.clone())?;
        Ok(RootedBuildAnalysis {
            source_root: self.declarations.snapshot, preparation: self.identity(),
            key: BuildAnalysisKey(self.canonical.build_lookup.digest), phase: canonical.phase,
            origin, receipt_written, diagnostics, analysis_elapsed, authored_projection_elapsed: start.elapsed(),
        })
    }

    fn project_diagnostics(&self, mut diagnostics: Vec<Diagnostic>) -> Result<Vec<Diagnostic>> {
        if diagnostics.is_empty() { return Ok(diagnostics); }
        let mut maps = HashMap::new();
        let tx = self.store.tree.begin_with_mode(Mode::ReadOnly)?;
        for diagnostic in &mut diagnostics {
            for label in &mut diagnostic.labels {
                let ordinal = *self.canonical.owners.get(&label.span.source)
                    .ok_or("compiler diagnostic outside selected project")?;
                let module = &self.declarations.modules()[ordinal.0];
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
}

fn executable_owner(module: &ProjectDeclarationModule) -> SourceIdentity {
    SourceIdentity::new(
        module.source.project.clone(), module.source.package.clone(), module.source.path.clone(),
        module.file.executable.hex(),
    )
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
        let start = Instant::now();
        let Some(snapshot) = self.snapshot(root)? else { return Ok(None); };
        let snapshot = snapshot.configured()?;
        let layout = snapshot.config.profiles.get(profile)
            .ok_or("artifact profile is not present in the rooted source configuration")?
            .target_layout()?;
        let target_layout = ScalarTargetLayout {
            pointer_size: layout.pointer_size,
            pointer_alignment: layout.pointer_alignment,
        };
        let build_inputs = BuildInputs {
            config: snapshot.source.config,
            executable: snapshot.source.executable,
            dependencies: snapshot.source.dependencies,
        };
        let declarations = self.project_declarations_for_snapshot(snapshot).await?;
        let mut report = ProjectPreparationReport {
            projected_declarations: start.elapsed(),
            ..Default::default()
        };
        let identity = preparation_identity(declarations.entry_projection, build_inputs, profile, target_layout);

        let flight = self.prepared_projects.flights.acquire(identity);
        let _permit = flight.lock().await;
        let canonical = if let Some(canonical) = self.prepared_projects.get(identity) {
            report.reused_live_preparation = true;
            canonical
        } else {
            let start = Instant::now();
            let modules = declarations.modules();
            let owners: Vec<_> = modules.iter().map(executable_owner).collect();
            let mut inputs = Vec::with_capacity(owners.len());
            for (module, owner) in modules.iter().zip(&owners) {
                let body = self.projected_body(module.file.executable, module.body)
                    .await?.with_executable_owner(owner.clone());
                // Source publication already paired these exact namespace CST
                // occurrences to targets in declaration order. An extraction
                // rejection has diagnostics that stop ordinary preparation
                // before it can consume namespace/type inputs.
                let bindings = body.namespaces().zip(&module.imports).map(|(site, link)| {
                    ScalarNamespaceBinding {
                        binding: site.binding.clone(),
                        target: owners[link.target.0].clone(),
                        span: site.span,
                    }
                }).collect();
                inputs.push((body, bindings));
            }
            report.materialized_bodies = start.elapsed();
            let start = Instant::now();
            let outcome = match prepare_scalar_project(
                inputs, owners.clone(), ScalarPreparationInputs { target_layout },
            ) {
                Ok(prepared) => PreparationOutcome::Ready(prepared),
                Err(rejected) => PreparationOutcome::Rejected(rejected),
            };
            report.compiler_preparation = start.elapsed();
            let owners = owners.into_iter().enumerate()
                .map(|(ordinal, identity)| (identity, SourceNodeId(ordinal))).collect();
            let canonical = Arc::new(CanonicalPreparation {
                identity, target_layout, owners, outcome,
                build_lookup: Lookup::new(self.derivation_image(), build_inputs, declarations.entry_projection, identity),
                build_analysis: OnceCell::new(),
            });
            self.prepared_projects.remember(identity, &canonical);
            canonical
        };
        Ok(Some(RootedPreparedProject { store: self, declarations, canonical, report }))
    }
}
