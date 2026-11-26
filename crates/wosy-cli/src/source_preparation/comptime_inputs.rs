use super::PreparedComptimeWatches;
use crate::comptime_fragment::{write_compiler_evaluator_ir, StagedEvaluatorIr};
use std::io;
use wosy_compiler::{
    ClosedComptimeFragment, ComptimeParameterUse, DirectStartupComptimeUse, ForwardedPairStartupComptimePlan,
    PairStartupForwarding, ScalarTargetLayout, ScalarType,
};
use wosy_project::{packages::ComptimeWatchLimits, SelectedComptimeTools};
use wosy_protocol::NodeId;

pub struct ClosedComptimeInputSelection<'a> {
    pub target: &'a str,
    pub profile: &'a str,
    pub owner: &'a str,
    pub id: &'a str,
    pub evaluator_layout: ScalarTargetLayout,
    pub watch_limits: ComptimeWatchLimits,
    pub max_ir_bytes: usize,
}

/// Observed facts for one selected candidate. They are neither a reusable
/// key nor an authorization: missing are the resolved executable/toolchain
/// closure, launch ABI, capability environment, and actual reachability.
/// In particular the authored toolchain label is not executable identity.
#[derive(Debug, Eq, PartialEq)]
pub struct ClosedComptimeInputFacts {
    source_root: NodeId,
    authored_cst: NodeId,
    configuration: String,
    watches: PreparedComptimeWatches,
    declared_tools: SelectedComptimeTools,
    evaluator_layout: ScalarTargetLayout,
    evaluator_ir: StagedEvaluatorIr,
    result_type: ScalarType,
    parameters: Vec<ComptimeParameterUse>,
}

impl ClosedComptimeInputFacts {
    pub(super) fn observe(
        source_root: NodeId, authored_cst: NodeId, configuration: String, watches: PreparedComptimeWatches,
        declared_tools: SelectedComptimeTools, fragment: ClosedComptimeFragment, max_ir_bytes: usize,
    ) -> Result<Self, String> {
        let evaluator_ir = write_compiler_evaluator_ir(&fragment, &mut io::sink(), max_ir_bytes)
            .map_err(|error| error.to_string())?;
        Ok(Self {
            source_root, authored_cst, configuration, watches, declared_tools,
            evaluator_layout: fragment.layout(),
            result_type: fragment.result_type().clone(),
            parameters: fragment.parameter_uses().to_vec(),
            evaluator_ir,
        })
    }

    pub fn source_root(&self) -> NodeId { self.source_root }
    pub fn authored_cst(&self) -> NodeId { self.authored_cst }
    pub fn configuration_identity(&self) -> &str { &self.configuration }
    pub fn watches(&self) -> &PreparedComptimeWatches { &self.watches }
    pub fn declared_tools(&self) -> &SelectedComptimeTools { &self.declared_tools }
    pub fn evaluator_layout(&self) -> ScalarTargetLayout { self.evaluator_layout }
    pub fn evaluator_ir(&self) -> &StagedEvaluatorIr { &self.evaluator_ir }
    pub fn result_type(&self) -> &ScalarType { &self.result_type }
    pub fn parameters(&self) -> &[ComptimeParameterUse] { &self.parameters }
}


pub struct ForwardedPairComptimeInputSelection<'a> {
    pub target: &'a str,
    pub profile: &'a str,
    pub id: &'a str,
    pub evaluator_layout: ScalarTargetLayout,
    pub watch_limits: ComptimeWatchLimits,
    pub max_ir_bytes: usize,
}

/// One ordinary consumer-owned startup observation, with authored positional
/// evidence and separate evaluator slots. No result/key, launcher permission,
/// executable identity, or guarantee that the watched files remain unchanged.
#[derive(Debug)]
pub struct ForwardedPairComptimeInputFacts {
    source_root: NodeId,
    authored_cst: NodeId,
    configuration: String,
    watches: PreparedComptimeWatches,
    declared_tools: SelectedComptimeTools,
    evaluator_layout: ScalarTargetLayout,
    forwarding: PairStartupForwarding,
    calls: Vec<DirectStartupComptimeUse<2>>,
    evaluator_irs: Vec<StagedEvaluatorIr>,
}

impl ForwardedPairComptimeInputFacts {
    pub(super) fn observe(
        source_root: NodeId, authored_cst: NodeId, configuration: String, watches: PreparedComptimeWatches,
        declared_tools: SelectedComptimeTools, plan: ForwardedPairStartupComptimePlan,
        max_ir_bytes: usize,
    ) -> Result<Self, String> {
        let (forwarding, plan) = plan.into_parts();
        let (calls, fragments) = plan.into_parts();
        let evaluator_layout = fragments.first().ok_or("forwarded comptime plan has no evaluator")?.layout();
        let mut evaluator_irs = Vec::with_capacity(fragments.len());
        let mut remaining = max_ir_bytes.min(1 << 20);
        for fragment in fragments {
            let ir = write_compiler_evaluator_ir(&fragment, &mut io::sink(), remaining)
                .map_err(|error| error.to_string())?;
            remaining -= ir.bytes;
            evaluator_irs.push(ir);
        }
        Ok(Self {
            source_root, authored_cst, configuration, watches, declared_tools, evaluator_layout,
            forwarding, calls, evaluator_irs,
        })
    }

    pub fn source_root(&self) -> NodeId { self.source_root }
    pub fn authored_cst(&self) -> NodeId { self.authored_cst }
    pub fn configuration_identity(&self) -> &str { &self.configuration }
    pub fn watches(&self) -> &PreparedComptimeWatches { &self.watches }
    pub fn declared_tools(&self) -> &SelectedComptimeTools { &self.declared_tools }
    pub fn evaluator_layout(&self) -> ScalarTargetLayout { self.evaluator_layout }
    pub fn forwarding(&self) -> &PairStartupForwarding { &self.forwarding }
    pub fn calls(&self) -> &[DirectStartupComptimeUse<2>] { &self.calls }
    pub fn evaluator_irs(&self) -> &[StagedEvaluatorIr] { &self.evaluator_irs }
}
