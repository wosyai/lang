//! Private local manifest for the selected WASIp1 tool pair.
//! Staging never authenticates tools, invokes them, or accepts a result.
use super::{provision::EvaluatorFiles, selected_builder_invocation, selected_runner_invocation, stdout::after_success,
    ClosedComptimeInvocation, ComptimeReadbackError, ProvisionedWasip1Comptime, StagedEvaluatorIr};
use crate::source_state::PendingPairComptimeInputObservation;
use serde::Serialize;
use std::{collections::BTreeMap, fmt, io::{self, Write}, path::Path, process::ExitStatus};
use wosy_compiler::{BoundedPairStartupWasip1Output, BoundedStartupWasip1Evaluator, DirectStartupComptimeUse,
    PairStartupBypass, PairStartupCallGraphEvidence, PairStartupCallPath, PairStartupOccurrence, ScalarExpression, ScalarType};
use wosy_project::SelectedComptimeTools;

#[derive(Serialize)]
struct ResolvedPrivateManifest<'a> {
    // Same path term as the ordinary local artifact manifest, not an
    // artifact identity and not an incremental wire payload.
    executable: &'a Path,
    parameters: BTreeMap<(), ()>,
    permissions: [(); 0],
    mounts: [(); 0],
}

pub struct StagedWasip1ToolPair {
    provision: ProvisionedWasip1Comptime,
}

impl ProvisionedWasip1Comptime {
    /// Materialize the one shared 3/1 manifest inside the existing owned
    /// scratch and retain the original prepared input. The P1 continuation
    /// has already admitted no build capabilities and no parameter calls.
    /// This first private handoff has no representation for authored
    /// parameter declarations. Other project paths remain unchanged.
    pub fn stage_tool_pair(self) -> io::Result<StagedWasip1ToolPair> {
        require_no_parameters(self.preparation().has_declared_parameters())?;
        write_private_manifest(self.create_private_manifest()?, self.expected_module_path())?;
        Ok(StagedWasip1ToolPair { provision: self })
    }
}

impl StagedWasip1ToolPair {
    pub fn provision(&self) -> &ProvisionedWasip1Comptime { &self.provision }
    pub fn builder(&self) -> ClosedComptimeInvocation<'_> { self.provision.builder_invocation() }
    pub fn runner(&self) -> ClosedComptimeInvocation<'_> { self.provision.runner_invocation() }
}


fn require_no_parameters(has_parameters: bool) -> io::Result<()> {
    if has_parameters {
        return Err(io::Error::new(io::ErrorKind::Unsupported, "the private WASIp1 pair needs empty resolved parameters"));
    }
    Ok(())
}

fn write_private_manifest(file: std::fs::File, module: &Path) -> io::Result<()> {
    let manifest = ResolvedPrivateManifest { executable: module, parameters: BTreeMap::new(), permissions: [], mounts: [] };
    serde_json::to_writer(file, &manifest).map_err(io::Error::other)
}

/// Disposable slots for the exact evaluator indices in the authenticated
/// local pending plan. This is not portable authority or a promise that the
/// selection, lease, files on disk, or tool executables remain current.
pub struct StagedPendingPairWasip1<'plan> {
    observation: PendingPairComptimeInputObservation,
    plan: &'plan BoundedPairStartupWasip1Output,
    files: Vec<EvaluatorFiles>,
}

pub struct StagedPendingPairWasip1Evaluator<'a> {
    project: &'a Path,
    tools: &'a SelectedComptimeTools,
    files: &'a EvaluatorFiles,
    evaluator: &'a BoundedStartupWasip1Evaluator,
}

impl<'plan> StagedPendingPairWasip1<'plan> {
    pub(crate) fn create(
        observation: PendingPairComptimeInputObservation, plan: &'plan BoundedPairStartupWasip1Output,
        private_parent: &Path, max_ir_bytes: usize,
    ) -> io::Result<Self> {
        require_no_parameters(!observation.declarations().parameters().is_empty())?;
        if plan.ir_bytes() > max_ir_bytes.min(1 << 20) {
            return Err(io::Error::other("closed comptime evaluator IR exceeds byte budget"));
        }
        let mut files = Vec::with_capacity(plan.evaluators().len());
        for evaluator in plan.evaluators() {
            let ir = evaluator.bytes();
            let own = EvaluatorFiles::create(private_parent, |file| {
                file.write_all(ir)?;
                Ok(StagedEvaluatorIr { bytes: ir.len(), blake3: blake3::hash(ir) })
            })?;
            write_private_manifest(own.create_manifest()?, own.module_path())?;
            files.push(own);
        }
        Ok(Self { observation, plan, files })
    }

    pub fn observation(&self) -> &PendingPairComptimeInputObservation { &self.observation }
    pub fn plan(&self) -> &BoundedPairStartupWasip1Output { self.plan }

    /// Inspect one already captured terminal per distinct staged evaluator,
    /// in evaluator-index order. Bytes/status are not launch provenance or
    /// permission to apply a result. A failed terminal yields no readback.
    pub fn read_stdout(&self, terminals: &[(ExitStatus, &[u8])])
        -> Result<PendingPairWasip1Readback<'_>, PendingPairWasip1ReadbackError>
    {
        let evaluators = self.plan.evaluators();
        let expected = evaluators.len();
        if terminals.len() != expected {
            return Err(PendingPairWasip1ReadbackError::TerminalCount { expected, supplied: terminals.len() });
        }
        let max_bytes = self.plan.max_stdout();
        let values = evaluators.iter().zip(terminals).enumerate().map(|(index, (evaluator, (status, bytes)))| {
            after_success(*status, || evaluator.parse_stdout(bytes, max_bytes))
                .map_err(|error| PendingPairWasip1ReadbackError::Evaluator { index, error })
        }).collect::<Result<_, _>>()?;
        Ok(PendingPairWasip1Readback { evidence: self.plan.evidence(), values })
    }

    pub fn evaluators(&self) -> impl ExactSizeIterator<Item = StagedPendingPairWasip1Evaluator<'_>> {
        let project = Path::new(&self.observation.binding().selected.project);
        let tools = self.observation.declarations().tools();
        self.files.iter().zip(self.plan.evaluators()).map(move |(files, evaluator)| {
            StagedPendingPairWasip1Evaluator { project, tools, files, evaluator }
        })
    }
}

impl StagedPendingPairWasip1Evaluator<'_> {
    pub fn evaluator_ir(&self) -> &StagedEvaluatorIr { self.files.ir() }
    pub fn result_type(&self) -> &ScalarType { self.evaluator.result_type() }
    pub fn manifest_path(&self) -> &Path { self.files.manifest_path() }
    pub fn expected_module_path(&self) -> &Path { self.files.module_path() }

    pub fn builder(&self) -> ClosedComptimeInvocation<'_> {
        selected_builder_invocation(self.project, self.tools, self.files.builder_paths())
    }

    pub fn runner(&self) -> ClosedComptimeInvocation<'_> {
        selected_runner_invocation(self.project, self.tools, self.manifest_path())
    }
}


/// Parsed inspection values on the same authored graph. Repeated call
/// slots borrow one parsed value; ordinary bypasses stay ordinary evidence.
pub struct PendingPairWasip1Readback<'stage> {
    evidence: &'stage PairStartupCallGraphEvidence,
    values: Vec<ScalarExpression>,
}

pub enum PendingPairWasip1Occurrence<'a> {
    Specialization {
        call: &'a DirectStartupComptimeUse<2>,
        path: &'a PairStartupCallPath,
        value: &'a ScalarExpression,
    },
    Bypass(&'a PairStartupBypass),
}

impl PendingPairWasip1Readback<'_> {
    pub fn occurrences(&self) -> impl ExactSizeIterator<Item = PendingPairWasip1Occurrence<'_>> {
        self.evidence.occurrences().map(|slot| match slot {
            PairStartupOccurrence::Specialization(call, path) => PendingPairWasip1Occurrence::Specialization {
                call, path, value: &self.values[call.evaluator_index()],
            },
            PairStartupOccurrence::Bypass(bypass) => PendingPairWasip1Occurrence::Bypass(bypass),
        })
    }
}

#[derive(Debug)]
pub enum PendingPairWasip1ReadbackError {
    TerminalCount { expected: usize, supplied: usize },
    Evaluator { index: usize, error: ComptimeReadbackError },
}

impl fmt::Display for PendingPairWasip1ReadbackError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TerminalCount { expected, supplied } =>
                write!(out, "comptime readback needs {expected} evaluator terminals, got {supplied}"),
            Self::Evaluator { index, error } => write!(out, "comptime evaluator {index}: {error}"),
        }
    }
}

impl std::error::Error for PendingPairWasip1ReadbackError {}
