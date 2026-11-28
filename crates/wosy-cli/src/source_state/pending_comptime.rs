//! Read-only client observation for a trusted local pending compiler plan.
use super::{checked_file_key, delta::pending_base_matches, read_selection, ClientSourceUsage,
    Result, SqliteSourceState, StoredNodes};
use crate::source_preparation::PreparedComptimeWatches;
use rusqlite::TransactionBehavior;
use std::path::Path;
use wosy_compiler::PairStartupCallGraphEvidence;
#[cfg(target_os = "linux")]
use {crate::comptime_fragment::{PendingPairWasip1Readback, StagedPendingPairWasip1}, std::{process::ExitStatus, time::{Duration, Instant}},
    wosy_compiler::BoundedPairStartupWasip1Output};
use wosy_project::{packages::{observe_owned_local_comptime_inputs, ComptimeWatchLimits, LocalComptimeInputScope, OwnedLocalComptimeInputs},
    ObservedClosedComptimeProfile};
use wosy_protocol::{BaseCstProposalBinding, FileEntry, NodeId, SourceConfig};

/// Borrow from the live prepared owner returned by an already trusted
/// local adapter. Not a portable or independently authenticated receipt.
pub struct TrustedPreparedPairComptime<'plan> {
    pub binding: &'plan BaseCstProposalBinding,
    pub authored_cst: NodeId,
    pub profile: &'plan str,
    pub graph: &'plan PairStartupCallGraphEvidence,
}

/// The same existing live local adapter, with the entire opaque compiler
/// graph + emitted P1 result binding from that owner, not independent bytes
/// or result types selected by the callback.
#[cfg(target_os = "linux")]
pub struct TrustedPreparedPairWasip1<'plan> {
    pub binding: &'plan BaseCstProposalBinding,
    pub authored_cst: NodeId,
    pub profile: &'plan str,
    pub output: &'plan BoundedPairStartupWasip1Output,
    pub execution_seal: &'plan wosy_project::packages::PreparedComptimeExecutionSeal,
}

/// Exact configuration and currently observed owning watch identities,
/// not executable/toolchain identity, a result key, or permission to launch.
pub struct PendingPairComptimeInputObservation {
    binding: BaseCstProposalBinding,
    authored_cst: NodeId,
    configuration_inputs: NodeId,
    owned: OwnedLocalComptimeInputs,
    watches: PreparedComptimeWatches,
}

impl PendingPairComptimeInputObservation {
    pub fn binding(&self) -> &BaseCstProposalBinding { &self.binding }
    pub fn authored_cst(&self) -> NodeId { self.authored_cst }
    pub fn configuration_inputs(&self) -> NodeId { self.configuration_inputs }
    pub fn declarations(&self) -> &ObservedClosedComptimeProfile { self.owned.declarations() }
    pub fn watches(&self) -> &PreparedComptimeWatches { &self.watches }
}

/// One actual local invocation of the exact selected pair. This receipt
/// does not prove hidden tool/environment inputs or publish compiler values.
#[cfg(target_os = "linux")]
pub struct ExecutedPendingPairWasip1<'stage> {
    readback: PendingPairWasip1Readback<'stage>,
    modules: Vec<blake3::Hash>,
    execution: wosy_project::packages::OwnedLocalComptimeExecution,
}

#[cfg(target_os = "linux")]
impl<'stage> ExecutedPendingPairWasip1<'stage> {
    pub fn execution(&self) -> &wosy_project::packages::OwnedLocalComptimeExecution { &self.execution }
    pub fn readback(&self) -> &PendingPairWasip1Readback<'stage> { &self.readback }
    pub fn modules(&self) -> &[blake3::Hash] { &self.modules }

}

impl SqliteSourceState {
    /// Join only the active dependency-free pending entry with the same
    /// authenticated in-process owner. A callback must borrow the graph and
    /// scope from that owner, never reinterpret caller-supplied compiler facts.
    /// No source bytes are reopened or parsed; SQLite reads end before any
    /// callback or filesystem observation, then the same attempt is checked.
    pub fn observe_pending_pair_comptime_inputs<'plan>(
        &mut self, binding: &BaseCstProposalBinding, profile: &str, watch_limits: ComptimeWatchLimits,
        trusted_plan: impl FnOnce() -> Option<TrustedPreparedPairComptime<'plan>>,
    ) -> Result<Option<PendingPairComptimeInputObservation>> {
        self.use_pending_pair_comptime_inputs(binding, profile, watch_limits,
            || trusted_plan().map(|plan| (plan, ())), |observation, ()| Ok(observation))
    }

    /// Materialize the 3/1 tool descriptions and their private slots only.
    /// The borrowed owner already compiled these exact P1 evaluators. No
    /// command runs, no new source is parsed, and no result is accepted.
    #[cfg(target_os = "linux")]
    pub fn stage_pending_pair_wasip1<'plan>(
        &mut self, binding: &BaseCstProposalBinding, profile: &str, watch_limits: ComptimeWatchLimits,
        private_parent: &Path, max_ir_bytes: usize,
        trusted_plan: impl FnOnce() -> Option<TrustedPreparedPairWasip1<'plan>>,
    ) -> Result<Option<StagedPendingPairWasip1<'plan>>> {
        self.use_pending_pair_comptime_inputs(binding, profile, watch_limits, || trusted_plan().map(|plan| {
            (TrustedPreparedPairComptime {
                binding: plan.binding, authored_cst: plan.authored_cst, profile: plan.profile, graph: plan.output.evidence(),
            }, (plan.output, plan.execution_seal))
        }), |observation, (output, seal)| Ok(StagedPendingPairWasip1::create(observation, output, seal.clone(), private_parent, max_ir_bytes)?))
    }

    /// Observe the supplied terminals only while the same attempt and inputs
    /// remain current. A successful result is neither execution provenance
    /// nor an immutable result root; no state is published.
    #[cfg(target_os = "linux")]
    pub fn observe_staged_pending_pair_wasip1<'stage>(
        &mut self, staging: &'stage StagedPendingPairWasip1<'_>,
        watch_limits: ComptimeWatchLimits, terminals: &[(ExitStatus, &[u8])],
    ) -> Result<Option<PendingPairWasip1Readback<'stage>>> {
        let original = staging.observation();
        let binding = original.binding();
        let profile = original.declarations().tools().profile();
        self.use_pending_pair_comptime_inputs(binding, profile, watch_limits, || Some((
            TrustedPreparedPairComptime { binding, authored_cst: original.authored_cst(), profile, graph: staging.plan().evidence() },
            (),
        )), |current, ()| {
            if current.watches() != original.watches() {
                return Err("local comptime inputs changed after evaluator staging".into());
            }
            Ok(staging.read_stdout(terminals)?)
        })
    }

    /// Execute the already-selected ordinary comptime commands exactly as
    /// configured, without a shell or caller-supplied terminal. The active
    /// source and declared watches must still match at both observations.
    #[cfg(target_os = "linux")]
    pub fn execute_staged_pending_pair_wasip1<'stage>(
        &mut self, staging: &'stage StagedPendingPairWasip1<'_>,
        watch_limits: ComptimeWatchLimits, max_module_bytes: u64, tool_timeout: Duration,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<ExecutedPendingPairWasip1<'stage>>> {
        let original = staging.observation();
        let binding = original.binding();
        let profile = original.declarations().tools().profile();
        if Path::new(&binding.selected.project) != self.project || !self.pending_base_is_current(binding)? { return Ok(None); }
        let deadline = Instant::now().checked_add(tool_timeout).ok_or("comptime tool deadline overflow")?;
        let execution = wosy_project::packages::execute_observed_comptime_set(
            &self.project, LocalComptimeInputScope {
                target: &binding.selected.target, profile, site: original.watches().id.as_str(), watch_limits,
            }, original.configuration_inputs().0,
            wosy_project::packages::ComptimeExecutionAttempt {
                seal: staging.execution_seal().clone(),
                root: binding.proposed.0, generation: binding.attempt, authored_cst: original.authored_cst().0,
            }, &original.watches().files, &staging.execution_stages(max_module_bytes), deadline, &cancelled,
        )?;
        let terminals = execution.pairs().iter().map(|pair| (pair.runner().status(), pair.runner().stdout())).collect::<Vec<_>>();
        if cancelled() || !self.pending_base_is_current(binding)? { return Ok(None); }
        let readback = staging.read_stdout(&terminals)?;
        Ok(Some(ExecutedPendingPairWasip1 {
            readback, modules: execution.pairs().iter().map(|pair| pair.module()).collect(), execution,
        }))
    }

    /// Check the attempt before borrowing trusted facts, and again after
    /// the watch/optional scratch operation. An obsolete completion drops
    /// the staged private files; the SQLite transaction does not span I/O.
    fn use_pending_pair_comptime_inputs<'plan, T, R>(
        &mut self, binding: &BaseCstProposalBinding, profile: &str, watch_limits: ComptimeWatchLimits,
        trusted_plan: impl FnOnce() -> Option<(TrustedPreparedPairComptime<'plan>, T)>,
        finish: impl FnOnce(PendingPairComptimeInputObservation, T) -> Result<R>,
    ) -> Result<Option<R>> {
        let Some((configuration, file)) = self.pending_pair_comptime_entry(binding)? else { return Ok(None); };
        if !configuration.profiles.contains_key(profile) {
            return Err("profile is not selected by the pending source".into());
        }
        let Some((plan, payload)) = trusted_plan() else { return Ok(None); };
        let site = plan.graph.site();
        let source = &file.metadata.source;
        if plan.binding != binding || plan.profile != profile || plan.authored_cst != file.authored
            || site.source.project != source.project || site.source.package != source.package
            || source.path.as_path().to_str() != Some(site.source.path.as_str())
            || NodeId::parse_hex(&site.source.revision)? != file.metadata.revision
        {
            return Err("local comptime plan does not match the exact pending source or profile".into());
        }
        let observed = observe_owned_local_comptime_inputs(&self.project, LocalComptimeInputScope {
            target: &binding.selected.target, profile, site: plan.graph.site_id(), watch_limits,
        }, configuration.inputs.0);
        let completed: Result<R> = observed.map_err(Into::into).and_then(|owned| {
            let watches = PreparedComptimeWatches {
                target: binding.selected.target.clone(), profile: profile.to_owned(), id: plan.graph.site_id().to_owned(),
                site: site.clone(), files: owned.watches().to_vec(),
            };
            finish(PendingPairComptimeInputObservation {
                binding: binding.clone(), authored_cst: file.authored,
                configuration_inputs: configuration.inputs, owned, watches,
            }, payload)
        });
        if !self.pending_base_is_current(binding)? { return Ok(None); }
        completed.map(Some)
    }

    fn pending_pair_comptime_entry(&mut self, binding: &BaseCstProposalBinding) -> Result<Option<(SourceConfig, FileEntry)>> {
        if Path::new(&binding.selected.project) != self.project { return Ok(None); }
        let tx = self.db.transaction_with_behavior(TransactionBehavior::Deferred)?;
        if !pending_base_matches(&read_selection(&tx, &binding.selected.target)?, binding) { return Ok(None); }
        let (configuration, file) = {
            let mut store = StoredNodes::new(&tx)?;
            let mut usage = ClientSourceUsage { nodes: 0, encoded_bytes: 0 };
            let mut read = |id| store.charged_record(id, false, |bytes| {
                usage.nodes += 1;
                usage.encoded_bytes = usage.encoded_bytes.saturating_add(bytes);
                Ok(usage.admit(self.limits)?)
            });
            let (configuration, ids) =
                Self::header_using(&self.project, &binding.selected.target, binding.proposed, &mut read)?;
            let [file] = ids.as_slice() else { return Err("local comptime observation requires a singleton entry".into()); };
            if configuration.namespaces.len() != 1 {
                return Err("local comptime input observation requires a dependency-free consumer".into());
            }
            let file = FileEntry::from_node(*file, read(*file)?)?;
            if checked_file_key(&configuration, configuration.parser.id(), &file)? != configuration.entry {
                return Err("local comptime entry does not match the pending source index".into());
            }
            (configuration, file)
        };
        tx.commit()?;
        Ok(Some((configuration, file)))
    }
}
