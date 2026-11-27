//! Read-only client observation for a trusted local pending compiler plan.
use super::{checked_file_key, delta::pending_base_matches, read_selection, ClientSourceUsage,
    Result, SqliteSourceState, StoredNodes};
use crate::source_preparation::{source_configuration_inputs, PreparedComptimeWatches};
use rusqlite::TransactionBehavior;
use std::{collections::BTreeMap, path::Path};
use wosy_compiler::PairStartupCallGraphEvidence;
#[cfg(target_os = "linux")]
use {crate::comptime_fragment::{PendingPairWasip1Readback, StagedPendingPairWasip1}, std::process::ExitStatus,
    wosy_compiler::BoundedPairStartupWasip1Output};
use wosy_project::{packages::{observe_local_comptime_inputs, ComptimeWatchLimits, LocalComptimeInputScope},
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
}

/// Exact configuration and currently observed owning watch identities,
/// not executable/toolchain identity, a result key, or permission to launch.
pub struct PendingPairComptimeInputObservation {
    binding: BaseCstProposalBinding,
    authored_cst: NodeId,
    configuration_inputs: NodeId,
    declarations: ObservedClosedComptimeProfile,
    watches: PreparedComptimeWatches,
}

impl PendingPairComptimeInputObservation {
    pub fn binding(&self) -> &BaseCstProposalBinding { &self.binding }
    pub fn authored_cst(&self) -> NodeId { self.authored_cst }
    pub fn configuration_inputs(&self) -> NodeId { self.configuration_inputs }
    pub fn declarations(&self) -> &ObservedClosedComptimeProfile { &self.declarations }
    pub fn watches(&self) -> &PreparedComptimeWatches { &self.watches }
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
            }, plan.output)
        }), |observation, output| Ok(StagedPendingPairWasip1::create(observation, output, private_parent, max_ir_bytes)?))
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
        let observed = observe_local_comptime_inputs(&self.project, LocalComptimeInputScope {
            target: &binding.selected.target, profile, site: plan.graph.site_id(), watch_limits,
        }, |manifest| {
            if source_configuration_inputs(manifest, &BTreeMap::new())? != configuration.inputs {
                return Err("local comptime configuration changed after source staging".into());
            }
            Ok(())
        });
        let completed: Result<R> = observed.map_err(Into::into).and_then(|(declarations, files)| {
            let watches = PreparedComptimeWatches {
                target: binding.selected.target.clone(), profile: profile.to_owned(), id: plan.graph.site_id().to_owned(),
                site: site.clone(), files,
            };
            finish(PendingPairComptimeInputObservation {
                binding: binding.clone(), authored_cst: file.authored,
                configuration_inputs: configuration.inputs, declarations, watches,
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
