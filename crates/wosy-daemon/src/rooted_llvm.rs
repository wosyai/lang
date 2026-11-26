//! Opt-in LLVM projections from one grammar-verified leased source root.
use super::{
    build_session::BuildRequestRejection, file_source, project_declarations::RootedOrder,
    grammar_receipts::VerifiedGrammarBatch,
    read_index_using, read_verified_node_with, retention_plan::SourceLease,
    rooted_llvm_receipts::{ExecutableLlvmFlight, ExecutableLlvmKey, ExecutableLlvmReceipt, FlightState, StagedExecutableLlvmReceipt},
    source_proposal::{authored_grammar_matches, take},
    source_reconciliation::{bounded_source_records, checked_source_header_using},
    storage_worker::{QueryPhases, QueryWorker},
    validate_file_using, BaseCstAdmissionLimits, FileTree, Result, SourceStore,
};
use std::{collections::{BTreeMap, HashMap}, io::{self, Write}, sync::Arc, time::{Duration, Instant}};
use surrealkv::Mode;
use tokio::{sync::{Mutex, OwnedMutexGuard}, time::{sleep_until, Instant as TokioInstant}};
use wosy_compiler::{
    analyze_scalar_build, derive_scalar_declarations_from_cst, prepare_scalar_project, Diagnostic,
    ScalarArtifactEmission, ScalarBuildValidation, ScalarNamespaceBinding, ScalarPreparationInputs, ScalarTargetLayout,
};
use wosy_protocol::{source_preparation_parts, CstKindEncoding, NodeId, OrdinaryLlvmBuilderWitness, ParserPolicy};
use wosy_syntax::{AuthoredSpanMap, ByteSpan, CanonicalCstRoot, CstNode, SourceIdentity};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RootedLlvmIdentity {
    /// Exact symbol identity of the existing ordinary Build.
    Authored,
    /// Use the verified executable CST's identity and omit typed-comment tokens.
    /// This changes internal legacy symbols; authored diagnostics are preserved.
    Executable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RootedLlvmOrigin { Derived, LiveExecutableReceipt }

impl RootedLlvmIdentity {
    fn tree(self) -> FileTree {
        match self {
            Self::Authored => FileTree::Authored,
            Self::Executable => FileTree::Both,
        }
    }

    fn owner(self, authored: SourceIdentity, executable: NodeId) -> (SourceIdentity, Option<SourceIdentity>) {
        match self {
            Self::Authored => (authored, None),
            Self::Executable => (
                SourceIdentity { revision: executable.hex(), ..authored.clone() }, Some(authored),
            ),
        }
    }
}

#[derive(Debug)]
pub enum RootedLlvmProjection {
    Unsupported(RootedLlvmUnsupported),
    Rejected(RootedLlvmRejection),
    Emitted(RootedLlvmPartition),
}

#[derive(Debug, Eq, PartialEq)]
pub enum RootedLlvmUnsupported {
    ProfileNotSelected,
    ProfileNotSupported(String),
    LegacySourcePath,
    CompilerImageUnavailable,
    AuthoredGrammarNotVerified,
}

#[derive(Debug)]
pub enum RootedLlvmRejection {
    Declarations(Vec<Diagnostic>),
    Preparation(Vec<Diagnostic>),
    Analysis(Vec<Diagnostic>),
    Emitter(String),
}

/// Exact partition bytes in the selected codegen identity, bound to the
/// leased source and actual compiler image. No artifact manifest, reuse
/// decision or semantic SnapshotRoot is implied.
#[derive(Debug)]
pub struct RootedLlvmPartition {
    pub source_root: NodeId,
    pub profile: String,
    pub compiler_image: NodeId,
    pub identity: RootedLlvmIdentity,
    pub origin: RootedLlvmOrigin,
    pub bytes: Vec<u8>,
    pub digest: NodeId,
    /// All files with verified grammar, counting a reused exact receipt.
    pub grammar_verified_files: usize,
    pub grammar_reparsed_files: usize,
    pub grammar_verification: Duration,
}

#[derive(Default)]
struct ProjectionEvidence {
    grammar: VerifiedGrammarBatch,
    executable: Option<ExecutableLlvmKey>,
}

pub(super) enum ScalarPlanning {
    Ready(ScalarPlan),
    Unsupported(RootedLlvmUnsupported),
}

pub(super) struct ScalarPlan {
    root: NodeId,
    profile: String,
    exports: Option<BTreeMap<String, String>>,
    backend: String,
    output: String,
    layout: ScalarTargetLayout,
    parser: ParserPolicy,
    identity: RootedLlvmIdentity,
    image: NodeId,
    workspace: String,
    order: RootedOrder,
    receipt_key: Option<ExecutableLlvmKey>,
}

pub(super) struct ScalarObservation<T> {
    pub outcome: ScalarOutcome<T>,
    pub root: NodeId,
    pub profile: String,
    pub image: NodeId,
    pub workspace: String,
    module_count: usize,
    grammar_reparsed_files: usize,
    grammar_verification: Duration,
}

pub(super) enum ScalarOutcome<T> {
    Rejected(RootedLlvmRejection),
    Unsupported(RootedLlvmUnsupported),
    Value(T),
    AlreadyEmitted,
}

/// One bounded existing worker slot across both stages. A waiting request
/// retains its own immutable read/transaction, but no blocking thread.
pub(super) async fn project_executable_on_worker(
    worker: QueryWorker, lease: Arc<SourceLease>, profile: String, limits: BaseCstAdmissionLimits, output_bytes: usize,
) -> Result<RootedLlvmProjection> {
    let grant = lease.grant();
    let (tx, read, plan) = worker.phase(None, move |store| {
        let (tx, read) = store.begin_leased_source(grant, Mode::ReadOnly)?.ok_or(BuildRequestRejection::Expired)?;
        let plan = store.plan_scalar(&tx, grant.root, profile, limits, RootedLlvmIdentity::Executable)?;
        Ok((tx, read, plan))
    }).await?;
    let plan = match plan {
        ScalarPlanning::Ready(plan) => plan,
        ScalarPlanning::Unsupported(reason) => {
            lease.finish_current(|| ()).ok_or(BuildRequestRejection::Expired)?;
            return Ok(RootedLlvmProjection::Unsupported(reason));
        }
    };
    let key = plan.receipt_key.expect("executable LLVM plan has an exact key");
    let phases = QueryPhases::new(&worker.store, Some(&worker), Some(&read));
    let mut receipt = worker.store.llvm_receipts.get(key);
    let mut producer = None;
    if receipt.is_none() {
        // If the bounded flight retention slots are occupied by distinct
        // keys, this request may still derive normally without sharing.
        if let Some(gate) = worker.store.llvm_flights.try_acquire_with(key, || worker.store.llvm_receipts.reserve_flight()) {
            let mut flight = acquire_llvm_flight(phases, &lease, Arc::clone(&gate)).await?;
            match &flight.state {
                FlightState::Complete(positive) => receipt = Some(Arc::clone(positive)),
                FlightState::Bypass => {}
                FlightState::Producing => {
                    if let Some(found) = worker.store.llvm_receipts.get(key) {
                        flight.state = FlightState::Complete(Arc::clone(&found));
                        worker.store.llvm_flights.detach(&key, &gate);
                        receipt = Some(found);
                    } else {
                        producer = Some((gate, flight));
                    }
                }
            }
        }
    }
    phases.run(move |store| {
        let mut producer = producer;
        lease.finish_current(|| ()).ok_or(BuildRequestRejection::Expired)?;
        let mut evidence = ProjectionEvidence::default();
        let result = store.project_planned_ll(&tx, plan, limits, output_bytes, receipt, &mut evidence)?;
        let admitted = store.finish_ll(&lease, &result, evidence)?;
        if let Some((gate, flight)) = producer.as_mut() {
            if matches!(result, RootedLlvmProjection::Emitted(_)) {
                // Move only the exact memo-admitted Arc. Oversize output
                // completes normally but hands no authority to the waiters.
                // Failed producers leave the gate available for a new try.
                flight.state = admitted.map_or(FlightState::Bypass, FlightState::Complete);
                store.llvm_flights.detach(&key, gate);
            }
        }
        Ok(result)
    }).await
}

async fn acquire_llvm_flight(
    phases: QueryPhases<'_>, lease: &SourceLease, gate: Arc<Mutex<ExecutableLlvmFlight>>,
) -> Result<OwnedMutexGuard<ExecutableLlvmFlight>> {
    let mut acquire = std::pin::pin!(phases.lock_owned(gate));
    loop {
        let deadline = lease.deadline().ok_or(BuildRequestRejection::Expired)?;
        if Instant::now() >= deadline { return Err(BuildRequestRejection::Expired.into()); }
        tokio::select! {
            biased;
            _ = sleep_until(TokioInstant::from_std(deadline)) => {
                // An owner may renew the lease while we wait. Continue the
                // same FIFO wait only when its current clock permits it.
            }
            result = &mut acquire => return result,
        }
    }
}

impl SourceStore {
    pub(super) fn project_ll_for_lease(
        &self, lease: &SourceLease, profile: String, limits: BaseCstAdmissionLimits, output_bytes: usize,
        identity: RootedLlvmIdentity,
    ) -> Result<RootedLlvmProjection> {
        let grant = lease.grant();
        let (tx, _read) = self.begin_leased_source(grant, Mode::ReadOnly)?
            .ok_or(BuildRequestRejection::Expired)?;
        let mut evidence = ProjectionEvidence::default();
        let result = self.project_ll(&tx, grant.root, profile, limits, output_bytes, identity, &mut evidence)?;
        drop(self.finish_ll(lease, &result, evidence)?);
        Ok(result)
    }

    /// The framed authored projection is a no-publication route. Even
    /// newly parsed grammar evidence is not installed by this request.
    pub(super) fn project_framed_authored_ll(
        &self, lease: &SourceLease, profile: String, limits: BaseCstAdmissionLimits,
        output_bytes: usize, builder_inputs: bool, check_active: impl Fn() -> Result<()>,
    ) -> Result<(RootedLlvmProjection, Option<OrdinaryLlvmBuilderWitness>)> {
        let grant = lease.grant();
        let (tx, _read) = self.begin_leased_source(grant, Mode::ReadOnly)?
            .ok_or(BuildRequestRejection::Expired)?;
        let (answer, builder) = match self.plan_scalar(&tx, grant.root, profile, limits, RootedLlvmIdentity::Authored)? {
            ScalarPlanning::Unsupported(reason) => (RootedLlvmProjection::Unsupported(reason), None),
            ScalarPlanning::Ready(plan) => {
                let witness = if builder_inputs {
                    let runtime = wosy_compiler::core_runtime(&plan.backend, &plan.output)?;
                    Some(OrdinaryLlvmBuilderWitness {
                        compiler: wosy_compiler::COMPILER_IDENTITY.into(),
                        llvm_adapter: wosy_compiler::LLVM_ADAPTER_IDENTITY.into(),
                        linked_llvm: wosy_compiler::linked_llvm_version(),
                        backend: plan.backend.clone(), output: plan.output.clone(),
                        runtime_digest: NodeId(*blake3::hash(runtime.as_bytes()).as_bytes()),
                    })
                } else { None };
                (self.project_planned_ll(&tx, plan, limits, output_bytes, None, &mut ProjectionEvidence::default())?, witness)
            }
        };
        lease.finish_current(check_active).ok_or(BuildRequestRejection::Expired)??;
        Ok((answer, builder))
    }

    fn finish_ll(
        &self, lease: &SourceLease, result: &RootedLlvmProjection, evidence: ProjectionEvidence,
    ) -> Result<Option<Arc<ExecutableLlvmReceipt>>> {
        if let RootedLlvmProjection::Emitted(projection) = result {
            let staged = evidence.executable.and_then(|key| {
                StagedExecutableLlvmReceipt::new(key, projection.digest, &projection.bytes)
            });
            self.finish_llvm_evidence(lease, evidence.grammar, staged)
        } else {
            lease.finish_current(|| ()).ok_or(BuildRequestRejection::Expired)?;
            Ok(None)
        }
    }

    fn finish_llvm_evidence(
        &self, lease: &SourceLease, grammar: VerifiedGrammarBatch,
        mut staged: Option<StagedExecutableLlvmReceipt>,
    ) -> Result<Option<Arc<ExecutableLlvmReceipt>>> {
        // No other memo operation acquires the source registry. Take the
        // needed memo locks first so lease admission can linearize both
        // zero-copy inserts without ever waiting on them under its gate.
        let mut grammar_memo = (!grammar.is_empty()).then(|| self.grammar_receipts.admission());
        let mut llvm_memo = staged.as_ref().map(|_| self.llvm_receipts.admission());
        let accepted = lease.finish_current(|| {
            if let Some(memo) = grammar_memo.as_mut() { memo.accept(grammar); }
            llvm_memo.as_mut().map(|memo| memo.accept(staged.take().expect("staged LLVM receipt")))
        });
        drop(llvm_memo);
        drop(grammar_memo);
        // Keep unadmitted or retired blobs alive until all gates are free.
        drop(staged);
        let admitted = accepted.ok_or(BuildRequestRejection::Expired)?;
        Ok(admitted.and_then(|(receipt, retired)| { drop(retired); receipt }))
    }

    fn project_ll(
        &self, tx: &surrealkv::Transaction, root: NodeId, profile: String,
        limits: BaseCstAdmissionLimits, output_bytes: usize, identity: RootedLlvmIdentity,
        evidence: &mut ProjectionEvidence,
    ) -> Result<RootedLlvmProjection> {
        match self.plan_scalar(tx, root, profile, limits, identity)? {
            ScalarPlanning::Ready(plan) => {
                let receipt = plan.receipt_key.and_then(|key| self.llvm_receipts.get(key));
                self.project_planned_ll(tx, plan, limits, output_bytes, receipt, evidence)
            }
            ScalarPlanning::Unsupported(reason) => Ok(RootedLlvmProjection::Unsupported(reason)),
        }
    }

    pub(super) fn plan_scalar(
        &self, tx: &surrealkv::Transaction, root: NodeId, profile: String,
        limits: BaseCstAdmissionLimits, identity: RootedLlvmIdentity,
    ) -> Result<ScalarPlanning> {
        use RootedLlvmUnsupported as Unsupported;

        let mut imports = limits.graph_edges;
        let mut read_index_node = bounded_source_records(tx, limits);
        let (top, mut config) = checked_source_header_using(root, &mut read_index_node)?;
        let parts = source_preparation_parts(&top).expect("checked header");
        let (config_id, executable, dependencies) = (parts.config, parts.executable, parts.dependencies);
        let workspace = super::pins::workspace_of_source_config(&config)?.to_owned();
        let Some(artifact) = config.profiles.remove(&profile) else {
            return Ok(ScalarPlanning::Unsupported(Unsupported::ProfileNotSelected));
        };
        let layout = match artifact.target_layout() {
            Ok(layout) => layout,
            Err(reason) => return Ok(ScalarPlanning::Unsupported(Unsupported::ProfileNotSupported(reason))),
        };
        let Some(image) = self.derivation_image() else {
            return Ok(ScalarPlanning::Unsupported(Unsupported::CompilerImageUnavailable));
        };
        let index = read_index_using(top, read_index_node, |count| {
            take(&mut imports, count, "LLVM source index exceeds the import budget")
        })?;
        // The admitted index fixes the same first-discovery order and
        // ordered destinations as the ordinary project source loader.
        let order = RootedOrder::from_parts(&config.entry, index.modules, index.files, index.module_edges);
        let receipt_key = (identity == RootedLlvmIdentity::Executable).then(|| {
            ExecutableLlvmKey::new(image, config_id, executable, dependencies, &profile, &order)
        });
        let layout = ScalarTargetLayout { pointer_size: layout.pointer_size, pointer_alignment: layout.pointer_alignment };
        Ok(ScalarPlanning::Ready(ScalarPlan {
            root, profile, exports: artifact.exports, backend: artifact.backend, output: artifact.output, layout,
            parser: config.parser, identity, image, workspace, order, receipt_key,
        }))
    }

    fn project_planned_ll(
        &self, tx: &surrealkv::Transaction, mut plan: ScalarPlan, limits: BaseCstAdmissionLimits, output_bytes: usize,
        receipt: Option<Arc<ExecutableLlvmReceipt>>, evidence: &mut ProjectionEvidence,
    ) -> Result<RootedLlvmProjection> {
        use RootedLlvmProjection as Projection;
        let exports = plan.exports.take();
        let identity = plan.identity;
        let receipt_key = plan.receipt_key;
        let ordinary = self.observe_planned_scalar(tx, plan, limits,
                Some((receipt.as_deref(), &mut evidence.grammar)),
                |_, _| Ok(()), |owner, module_count, validation| {
                    let emission = ScalarArtifactEmission::select(
                        module_count, exports.as_ref().map(|exports| (owner, exports)),
                    );
                    let partition = match emission.emit(validation) {
                        Ok(partition) => partition,
                        Err(error) => return Ok(Err(error)),
                    };
                    let mut output = PartitionBytes { bytes: Vec::new(), limit: output_bytes };
                    partition.write_text(&mut output)?;
                    let digest = NodeId(*blake3::hash(&output.bytes).as_bytes());
                    Ok(Ok((output.bytes, digest)))
                })?;
        let project = |bytes, digest, origin| Projection::Emitted(RootedLlvmPartition {
            source_root: ordinary.root, profile: ordinary.profile, compiler_image: ordinary.image, identity, origin, bytes, digest,
            grammar_verified_files: ordinary.module_count, grammar_reparsed_files: ordinary.grammar_reparsed_files,
            grammar_verification: ordinary.grammar_verification,
        });
        match ordinary.outcome {
            ScalarOutcome::Unsupported(reason) => Ok(Projection::Unsupported(reason)),
            ScalarOutcome::Rejected(error) => Ok(Projection::Rejected(error)),
            ScalarOutcome::Value(Err(error)) => Ok(Projection::Rejected(RootedLlvmRejection::Emitter(error))),
            ScalarOutcome::Value(Ok((bytes, digest))) => {
                evidence.executable = receipt_key;
                Ok(project(bytes, digest, RootedLlvmOrigin::Derived))
            }
            ScalarOutcome::AlreadyEmitted => {
                let receipt = receipt.expect("no analysis implies an existing LLVM receipt");
                if receipt.bytes.len() > output_bytes { return Err(partition_budget_error().into()); }
                Ok(project(receipt.bytes.to_vec(), receipt.digest, RootedLlvmOrigin::LiveExecutableReceipt))
            }
        }
    }

    /// Same ordinary scalar preparation/analysis for emission and a fresh
    /// authored observation. Only the LLVM caller can supply a cached result.
    pub(super) fn observe_planned_scalar<T>(
        &self, tx: &surrealkv::Transaction, plan: ScalarPlan, limits: BaseCstAdmissionLimits,
        mut llvm_memo: Option<(Option<&ExecutableLlvmReceipt>, &mut VerifiedGrammarBatch)>,
        mut authored_tree: impl FnMut(&SourceIdentity, &CstNode) -> Result<()>,
        finish: impl FnOnce(&SourceIdentity, usize, &ScalarBuildValidation<'_>) -> Result<T>,
    ) -> Result<ScalarObservation<T>> {
        use RootedLlvmRejection as Rejection;
        use RootedLlvmUnsupported as Unsupported;

        let ScalarPlan { root, profile, layout, parser, identity, image, workspace, order, .. } = plan;
        let policy = parser.id();
        let module_count = order.modules.len();
        let mut grammar_verification = Duration::ZERO;
        let mut grammar_reparsed_files = 0;
        let complete = |outcome, grammar_reparsed_files, grammar_verification| Ok(ScalarObservation {
            outcome, root, profile, image, workspace, module_count, grammar_reparsed_files, grammar_verification,
        });
        let cache_hit = llvm_memo.as_ref().is_some_and(|(receipt, _)| receipt.is_some());
        let compile_modules = if !cache_hit { module_count } else { 0 };
        let mut owners = Vec::with_capacity(compile_modules);
        let mut original_owners = Vec::new();
        if identity == RootedLlvmIdentity::Executable { original_owners.reserve(compile_modules); }
        for module in &order.modules {
            if module.file.metadata.policy != policy {
                return Err("LLVM file does not match the selected parser policy".into());
            }
            if module.file.metadata.source.path.as_path().to_str().is_none_or(|path| path.contains('\\')) {
                // Legacy ordinary Build rewrites the display path. This is
                // not the same identity as the rooted native path.
                return complete(ScalarOutcome::Unsupported(Unsupported::LegacySourcePath), 0, Duration::ZERO);
            }
            if !cache_hit {
                let (owner, authored) = identity.owner(file_source(&module.file)?, module.file.executable);
                owners.push(owner);
                original_owners.extend(authored);
            }
        }
        let mut work = limits.cst_work();
        let mut input = Vec::with_capacity(compile_modules);
        let mut extracted = Vec::new();
        let mut authored_sources = HashMap::new();
        let mut original_owners = original_owners.into_iter();
        for (ordinal, module) in order.modules.into_iter().enumerate() {
            let grammar_known = llvm_memo.is_some() && self.grammar_receipts.contains(&module.file);
            let trees = match (cache_hit, grammar_known) {
                (true, true) => FileTree::None,
                (true, false) => FileTree::Authored,
                (false, _) => identity.tree(),
            };
            let checked = validate_file_using(&module.file, &parser.categories, trees, CstKindEncoding::Current,
                Some(&mut work), |id, work| {
                    read_verified_node_with(tx, id, |bytes| work.expect("bounded authored expansion").read(bytes))
                })?;
            // Hash, authored closure and budgets must pass even on receipt
            // hits. A generic published marker carries no grammar witness.
            if !grammar_known {
                let verification = Instant::now();
                if !authored_grammar_matches(&module.file, &parser.categories, &checked)? {
                    return complete(ScalarOutcome::Unsupported(Unsupported::AuthoredGrammarNotVerified), grammar_reparsed_files, grammar_verification);
                }
                grammar_verification += verification.elapsed();
                grammar_reparsed_files += 1;
                if let Some((_, grammar)) = llvm_memo.as_mut() { grammar.verified(&module.file); }
            }
            if cache_hit { continue; }
            let owner = &owners[ordinal];
            let selected = if let Some(original) = original_owners.next() {
                authored_sources.insert(owner.clone(), (original, AuthoredSpanMap::from_comments(&checked.comments)));
                checked.executable.ok_or("leased source has no executable CST")?
            } else {
                let tree = checked.authored.ok_or("leased source has no authored CST")?;
                authored_tree(owner, &tree)?;
                tree
            };
            let tree = CanonicalCstRoot::new(owner.clone(), selected);
            let declarations = derive_scalar_declarations_from_cst(&tree);
            extracted.extend_from_slice(declarations.diagnostics());
            let bindings = match identity {
                RootedLlvmIdentity::Authored => module.file.metadata.imports.into_iter().zip(module.imports)
                    .map(|(site, target)| ScalarNamespaceBinding {
                        binding: site.binding, target: owners[target.0].clone(),
                        span: ByteSpan::new(site.span.start, site.span.end),
                    }).collect(),
                RootedLlvmIdentity::Executable if declarations.diagnostics().is_empty() => {
                    // Publication already paired canonical namespace order
                    // to resolved destinations. Take coordinates from the
                    // actual executable compiler declarations, not the
                    // separately authored offset hints.
                    let mut resolved = module.imports.into_iter();
                    let mut bindings = Vec::with_capacity(resolved.len());
                    for site in declarations.namespaces() {
                        let target = resolved.next().ok_or("executable namespace has no resolved destination")?;
                        bindings.push(ScalarNamespaceBinding {
                            binding: site.binding.clone(), target: owners[target.0].clone(), span: site.span,
                        });
                    }
                    if resolved.next().is_some() { return Err("resolved destination has no executable namespace".into()); }
                    bindings
                }
                RootedLlvmIdentity::Executable => Vec::new(),
            };
            input.push((declarations, bindings));
        }
        let result = |outcome| complete(outcome, grammar_reparsed_files, grammar_verification);
        if cache_hit {
            // This branch never prepares compiler state and is available
            // only to LLVM projection, not to historical diagnostics.
            return result(ScalarOutcome::AlreadyEmitted);
        }
        let root_owner = owners.first().ok_or("leased source has no entry owner")?.clone();
        let authored = |diagnostics| project_authored(identity, &authored_sources, diagnostics);
        if !extracted.is_empty() { return result(ScalarOutcome::Rejected(Rejection::Declarations(authored(extracted)?))); }
        let prepared = match prepare_scalar_project(input, owners, ScalarPreparationInputs { target_layout: layout }) {
            Ok(prepared) => prepared,
            Err(rejected) => return result(ScalarOutcome::Rejected(Rejection::Preparation(authored(rejected.into_ordered_diagnostics())?))),
        };
        let validation = analyze_scalar_build(&prepared);
        if !validation.diagnostics().is_empty() {
            return result(ScalarOutcome::Rejected(Rejection::Analysis(authored(validation.diagnostics().to_vec())?)));
        }
        result(ScalarOutcome::Value(finish(&root_owner, module_count, &validation)?))

    }
}

fn project_authored(
    identity: RootedLlvmIdentity,
    sources: &HashMap<SourceIdentity, (SourceIdentity, AuthoredSpanMap)>,
    mut diagnostics: Vec<Diagnostic>,
) -> Result<Vec<Diagnostic>> {
    if identity == RootedLlvmIdentity::Executable {
        for label in diagnostics.iter_mut().flat_map(|diagnostic| &mut diagnostic.labels) {
            let (source, spans) = sources.get(&label.span.source)
                .ok_or("LLVM diagnostic lies outside the leased authored source")?;
            label.span.source = source.clone();
            label.span.range = spans.project(label.span.range);
        }
    }
    Ok(diagnostics)
}

// Limit retained egress, not temporary LLVM allocations during emission.
struct PartitionBytes {
    bytes: Vec<u8>,
    limit: usize,
}

impl Write for PartitionBytes {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit - self.bytes.len() {
            return Err(partition_budget_error());
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}

fn partition_budget_error() -> io::Error {
    io::Error::other("LLVM partition exceeds the output byte budget")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::{poll_fn, Future};
    use std::num::NonZeroUsize;
    use std::path::PathBuf;
    use std::task::Poll;
    use wosy_protocol::BuildSourceSelection;

    fn fixture_source() -> (PathBuf, BuildSourceSelection, BaseCstAdmissionLimits) {
        let path = PathBuf::from(std::env::var_os("WOSY_TEST_IMAGE_STORE").expect("owned fixture store"));
        let project = std::env::var("WOSY_TEST_LLVM_PROJECT").expect("owned fixture project");
        let root = NodeId::parse_hex(&std::env::var("WOSY_TEST_IMAGE_ROOT").expect("published fixture root")).unwrap();
        let selected = BuildSourceSelection { root, project, target: "app".into() };
        let limits = BaseCstAdmissionLimits {
            records: 25_000, record_bytes: 16 << 20, graph_edges: 25_000,
            expanded_record_bytes: 32 << 20, cst_occurrences: 200_000,
            cst_depth: 1024, authored_bytes: 8 << 20,
        };
        (path, selected, limits)
    }

    #[test]
    #[ignore = "requires a disposable published project source fixture"]
    fn closed_projection_cannot_install_grammar_or_llvm() {
        let (path, selected, limits) = fixture_source();
        let root = selected.root;
        let identity = RootedLlvmIdentity::Executable;
        tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap().block_on(async {
            let store = SourceStore::open(&path, 8 << 20).unwrap();
            let open = |duration| store.lease_build_source(selected.clone(), duration).unwrap().unwrap();
            for release in [true, false] {
                let lease = open(if release { Duration::from_secs(10) } else { Duration::from_secs(1) });
                let (tx, _read) = store.begin_leased_source(lease.grant(), Mode::ReadOnly).unwrap().unwrap();
                let mut evidence = ProjectionEvidence::default();
                let output = store.project_ll(&tx, root, "dev".into(), limits, 2 << 20, identity, &mut evidence).unwrap();
                let RootedLlvmProjection::Emitted(output) = output else { panic!("fixture must derive real LLVM"); };
                assert_eq!(output.origin, RootedLlvmOrigin::Derived);
                assert!(output.grammar_verified_files > 0);
                assert_eq!(output.grammar_reparsed_files, output.grammar_verified_files);
                let key = evidence.executable.expect("positive compiler receipt");
                let payload = StagedExecutableLlvmReceipt::new(key, output.digest, &output.bytes).unwrap();
                assert!(store.llvm_receipts.get(key).is_none());
                if release {
                    lease.release();
                } else {
                    let deadline = lease.deadline().unwrap();
                    tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
                    assert!(Instant::now() >= deadline);
                }
                let error = store.finish_llvm_evidence(&lease, evidence.grammar, Some(payload)).err().expect("closed lease");
                assert!(matches!(error.downcast_ref::<BuildRequestRejection>(), Some(BuildRequestRejection::Expired)));
                assert!(store.llvm_receipts.get(key).is_none());
            }

            let live = open(Duration::from_secs(10));
            let run = || store.project_ll_for_lease(&live, "dev".into(), limits, 2 << 20, identity).unwrap();
            let RootedLlvmProjection::Emitted(first) = run() else { panic!("live derivation missing"); };
            let RootedLlvmProjection::Emitted(second) = run() else { panic!("live receipt missing"); };
            assert_eq!((first.origin, first.grammar_reparsed_files), (RootedLlvmOrigin::Derived, first.grammar_verified_files));
            assert_eq!((second.origin, second.grammar_reparsed_files), (RootedLlvmOrigin::LiveExecutableReceipt, 0));
            assert_eq!(first.bytes, second.bytes);
            live.release();
            store.close().await.unwrap();
        });
    }

    #[test]
    #[ignore = "requires a disposable published annotation-only source pair"]
    fn executable_llvm_waiters_share_flight_and_release_slots() {
        let (path, a, limits) = fixture_source();
        let root = NodeId::parse_hex(&std::env::var("WOSY_TEST_IMAGE_OTHER_ROOT").expect("second fixture root")).unwrap();
        let b = BuildSourceSelection { root, ..a.clone() };
        assert_ne!(a.root, b.root);
        let identity = RootedLlvmIdentity::Executable;
        tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap().block_on(async {
            use crate::{SourceStorageWorker, StorageAdmissionError};
            use tokio::{runtime::Handle, sync::Semaphore, time::timeout};

            let store = Arc::new(SourceStore::open(&path, 8 << 20).unwrap());
            let bound = |value| NonZeroUsize::new(value).unwrap();
            let mut worker = SourceStorageWorker::new(Arc::clone(&store), Handle::current(), bound(4), bound(4), bound(25_000)).unwrap();
            let gate_waiters = Arc::new(Semaphore::new(0));
            let signalled = Arc::clone(&gate_waiters);
            worker.on_cancellable_gate_wait(move || signalled.add_permits(1));

            let open = |selection, time| worker.open_build_diagnostics(selection, time);
            let time = Duration::from_secs(15);
            let first = open(a.clone(), time).await.unwrap().unwrap();
            let second = open(b.clone(), time).await.unwrap().unwrap();
            let abandoned = open(b.clone(), time).await.unwrap().unwrap();
            let expiring = open(b.clone(), Duration::from_secs(2)).await.unwrap().unwrap();
            let gate = {
                let (tx, _read) = store.begin_source_read(a.root).unwrap().unwrap();
                let ScalarPlanning::Ready(plan) = store.plan_scalar(&tx, a.root, "dev".into(), limits, identity).unwrap()
                    else { panic!("fixture needs a valid LLVM plan"); };
                store.llvm_flights.try_acquire_with(plan.receipt_key.unwrap(), || store.llvm_receipts.reserve_flight()).unwrap()
            };
            let barrier = gate.lock().await;
            let mut first_result = Box::pin(first.project_llvm("dev".into(), limits, 2 << 20, identity));
            let mut second_result = Box::pin(second.project_llvm("dev".into(), limits, 2 << 20, identity));
            let mut abandoned_result = Box::pin(abandoned.project_llvm("dev".into(), limits, 2 << 20, identity));
            let mut expiring_result = Box::pin(expiring.project_llvm("dev".into(), limits, 2 << 20, identity));
            for result in [&mut first_result, &mut second_result, &mut abandoned_result, &mut expiring_result] {
                poll_fn(|cx| {
                    assert!(result.as_mut().poll(cx).is_pending());
                    Poll::Ready(())
                }).await;
            }
            timeout(Duration::from_secs(1), gate_waiters.acquire_many(4)).await.unwrap().unwrap().forget();
            assert!(matches!(worker.try_slot(), Err(StorageAdmissionError::Busy)));

            drop(abandoned_result);
            let freed = timeout(Duration::from_secs(1), async {
                loop {
                    if let Ok(slot) = worker.try_slot() { break slot; }
                    tokio::task::yield_now().await;
                }
            }).await.unwrap();
            assert!(matches!(worker.try_slot(), Err(StorageAdmissionError::Busy)));
            let expired = timeout(Duration::from_secs(4), expiring_result).await.unwrap().unwrap_err();
            assert!(matches!(expired.downcast_ref::<BuildRequestRejection>(), Some(BuildRequestRejection::Expired)));
            let freed_after_expiry = worker.try_slot().unwrap();
            assert!(matches!(worker.try_slot(), Err(StorageAdmissionError::Busy)));

            drop((freed, freed_after_expiry, barrier));
            let (first_result, second_result) = tokio::join!(first_result, second_result);
            let RootedLlvmProjection::Emitted(first_result) = first_result.unwrap() else { panic!("first projection missing"); };
            let RootedLlvmProjection::Emitted(second_result) = second_result.unwrap() else { panic!("second projection missing"); };
            assert_eq!((first_result.source_root, second_result.source_root), (a.root, b.root));
            assert_eq!(first_result.bytes, second_result.bytes);
            let origins = [first_result.origin, second_result.origin];
            assert_eq!(origins.iter().filter(|&&origin| origin == RootedLlvmOrigin::Derived).count(), 1);
            assert!(origins.contains(&RootedLlvmOrigin::LiveExecutableReceipt));
            assert_eq!(first_result.grammar_reparsed_files + second_result.grammar_reparsed_files,
                first_result.grammar_verified_files + 1, "both authored revisions are actually verified");
            drop((first, second, abandoned, expiring));
            drop(worker);
            Arc::try_unwrap(store).ok().unwrap().close().await.unwrap();
        });
    }

    #[test]
    #[ignore = "requires disposable base, annotation, executable and configuration revisions"]
    fn registered_executable_waiter_reuses_after_eviction_and_failed_producer() {
        let (path, a, limits) = fixture_source();
        let selected = |name| BuildSourceSelection { root: NodeId::parse_hex(&std::env::var(name).unwrap()).unwrap(), ..a.clone() };
        let b = selected("WOSY_TEST_IMAGE_OTHER_ROOT");
        let c = selected("WOSY_TEST_LLVM_EXECUTABLE_ROOT");
        let d = selected("WOSY_TEST_LLVM_CONFIGURED_ROOT");
        let identity = RootedLlvmIdentity::Executable;
        tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap().block_on(async {
            use crate::SourceStorageWorker;
            use tokio::{runtime::Handle, sync::Semaphore, time::timeout};
            let bound = |value| NonZeroUsize::new(value).unwrap();
            for (profile, other) in [("dev", "native"), ("native", "dev")] {
                // First use a new real memo. A too-small caller fails in the
                // ordinary writer; the already queued valid B must derive.
                let store = Arc::new(SourceStore::open(&path, 8 << 20).unwrap());
                let mut worker = SourceStorageWorker::new(Arc::clone(&store), Handle::current(), bound(4), bound(4), bound(25_000)).unwrap();
                let arrivals = Arc::new(Semaphore::new(0));
                let signal = Arc::clone(&arrivals);
                worker.on_cancellable_gate_wait(move || signal.add_permits(1));
                let wait_for_arrival = || async {
                    timeout(Duration::from_secs(3), arrivals.acquire()).await.unwrap().unwrap().forget();
                };
                let time = Duration::from_secs(30);
                let aa = worker.open_build_diagnostics(a.clone(), time).await.unwrap().unwrap();
                let bb = worker.open_build_diagnostics(b.clone(), time).await.unwrap().unwrap();
                let key = {
                    let (tx, _read) = store.begin_source_read(a.root).unwrap().unwrap();
                    let ScalarPlanning::Ready(plan) = store.plan_scalar(&tx, a.root, profile.into(), limits, identity).unwrap()
                        else { panic!("fixture needs a valid LLVM plan"); };
                    plan.receipt_key.unwrap()
                };
                let fresh_gate = || store.llvm_flights.try_acquire_with(key, || store.llvm_receipts.reserve_flight()).unwrap();
                let gate = fresh_gate();
                let barrier = gate.lock().await;
                let mut fail = Box::pin(aa.project_llvm(profile.into(), limits, 1, identity));
                poll_fn(|cx| { assert!(fail.as_mut().poll(cx).is_pending()); Poll::Ready(()) }).await;
                wait_for_arrival().await;
                let mut next = Box::pin(bb.project_llvm(profile.into(), limits, 2 << 20, identity));
                poll_fn(|cx| { assert!(next.as_mut().poll(cx).is_pending()); Poll::Ready(()) }).await;
                wait_for_arrival().await;
                drop(barrier);
                assert!(fail.await.unwrap_err().to_string().contains("output byte budget"));
                let RootedLlvmProjection::Emitted(next) = next.await.unwrap() else { panic!("valid successor needs LLVM"); };
                assert_eq!((next.origin, next.source_root), (RootedLlvmOrigin::Derived, b.root));
                assert_eq!(next.grammar_reparsed_files, next.grammar_verified_files);
                drop((gate, aa, bb));
                drop(worker);
                Arc::try_unwrap(store).ok().unwrap().close().await.unwrap();

                // Queue a fixture guard after the real leader but before B,
                // then derive four distinct real results while B stays queued.
                // The FIFO can no longer help when B finally wakes.
                let store = Arc::new(SourceStore::open(&path, 8 << 20).unwrap());
                let mut worker = SourceStorageWorker::new(Arc::clone(&store), Handle::current(), bound(4), bound(4), bound(25_000)).unwrap();
                let signal = Arc::clone(&arrivals);
                worker.on_cancellable_gate_wait(move || signal.add_permits(1));
                let aa = worker.open_build_diagnostics(a.clone(), time).await.unwrap().unwrap();
                let bb = worker.open_build_diagnostics(b.clone(), time).await.unwrap().unwrap();
                let gate = store.llvm_flights.try_acquire_with(key, || store.llvm_receipts.reserve_flight()).unwrap();
                let barrier = gate.lock().await;
                let mut leader = Box::pin(aa.project_llvm(profile.into(), limits, 2 << 20, identity));
                poll_fn(|cx| { assert!(leader.as_mut().poll(cx).is_pending()); Poll::Ready(()) }).await;
                wait_for_arrival().await;
                let mut interposer = Box::pin(Arc::clone(&gate).lock_owned());
                poll_fn(|cx| { assert!(interposer.as_mut().poll(cx).is_pending()); Poll::Ready(()) }).await;
                let mut follower = Box::pin(bb.project_llvm(profile.into(), limits, 2 << 20, identity));
                poll_fn(|cx| { assert!(follower.as_mut().poll(cx).is_pending()); Poll::Ready(()) }).await;
                wait_for_arrival().await;
                drop(barrier);
                let RootedLlvmProjection::Emitted(leader) = leader.await.unwrap() else { panic!("leader needs LLVM"); };
                assert_eq!(leader.origin, RootedLlvmOrigin::Derived);
                let interposer = interposer.await;
                let FlightState::Complete(accepted) = &interposer.state else { panic!("leader must own bounded positive receipt"); };
                assert!(Arc::ptr_eq(accepted, &store.llvm_receipts.get(key).unwrap()));
                for (scope, profile) in [(&a, other), (&c, profile), (&c, other), (&d, profile)] {
                    let session = worker.open_build_diagnostics(scope.clone(), time).await.unwrap().unwrap();
                    let output = session.project_llvm(profile.into(), limits, 2 << 20, identity).await.unwrap();
                    let RootedLlvmProjection::Emitted(output) = output else { panic!("eviction control must emit LLVM"); };
                    assert_eq!(output.origin, RootedLlvmOrigin::Derived);
                }
                assert!(store.llvm_receipts.get(key).is_none(), "real distinct projections must evict the original receipt");
                // A new registration cannot borrow this completed flight.
                let replacement = store.llvm_flights.try_acquire_with(key, || store.llvm_receipts.reserve_flight()).unwrap();
                assert!(!Arc::ptr_eq(&replacement, &gate));
                drop((replacement, interposer, gate));
                let RootedLlvmProjection::Emitted(follower) = follower.await.unwrap() else { panic!("registered follower needs LLVM"); };
                assert_eq!((follower.origin, follower.source_root), (RootedLlvmOrigin::LiveExecutableReceipt, b.root));
                assert_eq!((follower.bytes, follower.digest), (leader.bytes, leader.digest));
                assert_eq!(follower.grammar_reparsed_files, 1, "B's authored revision is still verified");
                assert!(store.llvm_receipts.get(key).is_none(), "handoff does not rewrite FIFO");
                drop((aa, bb));
                drop(worker);
                Arc::try_unwrap(store).ok().unwrap().close().await.unwrap();
            }
        });
    }
}
