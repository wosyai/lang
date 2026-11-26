use super::PreparedComptimeWatches;
use crate::comptime_fragment::{write_compiler_evaluator_ir, StagedEvaluatorIr};
use std::io;
use wosy_compiler::{ClosedComptimeFragment, ComptimeParameterUse, ScalarTargetLayout, ScalarType};
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
