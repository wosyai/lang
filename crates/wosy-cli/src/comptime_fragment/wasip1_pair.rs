//! Private local manifest for the selected WASIp1 tool pair.
//! Staging never authenticates tools, invokes them, or accepts a result.
use super::{provision::EvaluatorFiles, selected_builder_invocation, selected_runner_invocation, stdout::after_success,
    ClosedComptimeInvocation, ComptimeReadbackError, ProvisionedWasip1Comptime, StagedEvaluatorIr};
use crate::source_state::PendingPairComptimeInputObservation;
use std::{fmt, io::{self, Write}, path::Path, process::ExitStatus};
use wosy_compiler::{BoundedPairStartupWasip1Output, BoundedStartupWasip1Evaluator, DirectStartupComptimeUse,
    ObservedPairStartupWasip1, PairStartupBypass, PairStartupCallPath, PairStartupOccurrence, ScalarExpression, ScalarType};
use wosy_project::{packages::{empty_wasip1_comptime_manifest, ComptimeEvaluatorStage, PreparedComptimeExecutionSeal, StagedComptimePairInputs}, SelectedComptimeTools};

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

fn write_private_manifest(mut file: std::fs::File, module: &Path) -> io::Result<()> {
    file.write_all(&empty_wasip1_comptime_manifest(module)?)
}

/// Disposable slots for the exact evaluator indices in the authenticated
/// local pending plan. This is not portable authority or a promise that the
/// selection, lease, files on disk, or tool executables remain current.
pub struct StagedPendingPairWasip1<'plan> {
    observation: PendingPairComptimeInputObservation,
    plan: &'plan BoundedPairStartupWasip1Output,
    files: Vec<EvaluatorFiles>,
    execution_seal: PreparedComptimeExecutionSeal,
}

pub struct StagedPendingPairWasip1Evaluator<'a> {
    project: &'a Path,
    tools: &'a SelectedComptimeTools,
    files: &'a EvaluatorFiles,
    evaluator: &'a BoundedStartupWasip1Evaluator,
}

fn stage_pair_evaluators(
    plan: &BoundedPairStartupWasip1Output, private_parent: &Path, max_ir_bytes: usize,
) -> io::Result<Vec<EvaluatorFiles>> {
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
    Ok(files)
}

fn evaluator_stages<'a>(
    files: &'a [EvaluatorFiles], max_module_bytes: u64, max_stdout: usize,
) -> Vec<ComptimeEvaluatorStage<'a>> {
    files.iter().map(|file| ComptimeEvaluatorStage {
        scratch: file.scratch(),
        inputs: StagedComptimePairInputs {
            ir_bytes: file.ir().bytes, ir: file.ir().blake3, max_module_bytes, max_stdout,
        },
    }).collect()
}

/// Client-owned selected-source execution, using the same stage and exact
/// argument/watch ABI as the pending-source route.
pub(crate) fn execute_selected_pair_wasip1(
    plan: &BoundedPairStartupWasip1Output, project: &Path,
    scope: wosy_project::packages::LocalComptimeInputScope<'_>,
    expected_config: [u8; 32], attempt: wosy_project::packages::ComptimeExecutionAttempt,
    private_parent: &Path, max_ir_bytes: usize, max_module_bytes: u64,
    deadline: std::time::Instant, cancelled: &impl Fn() -> bool,
) -> io::Result<wosy_project::packages::OwnedLocalComptimeExecution> {
    use wosy_project::packages::{execute_observed_comptime_set, observe_owned_local_comptime_inputs, LocalComptimeInputScope};
    let staged = observe_owned_local_comptime_inputs(project, LocalComptimeInputScope {
        target: scope.target, profile: scope.profile, site: scope.site, watch_limits: scope.watch_limits,
    }, expected_config).map_err(io::Error::other)?;
    require_no_parameters(!staged.declarations().parameters().is_empty())?;
    let files = stage_pair_evaluators(plan, private_parent, max_ir_bytes)?;
    execute_observed_comptime_set(project, scope, expected_config, attempt, staged.watches(),
        &evaluator_stages(&files, max_module_bytes, plan.max_stdout()), deadline, cancelled)
}

/// Run exactly the selected closed site's one evaluator using the same
/// ordinary three-input builder and one-input runner as the paired route.
pub(crate) fn execute_selected_closed_wasip1(
    evaluator: &wosy_compiler::OrderedClosedWasip1Site, project: &Path,
    scope: wosy_project::packages::LocalComptimeInputScope<'_>,
    expected_config: [u8; 32], attempt: wosy_project::packages::ComptimeExecutionAttempt,
    watches: &[wosy_project::packages::WatchedComptimeFile],
    private_parent: &Path, max_module_bytes: u64,
    deadline: std::time::Instant, cancelled: &impl Fn() -> bool,
) -> io::Result<wosy_project::packages::OwnedLocalComptimeExecution> {
    let ir = evaluator.ir();
    let file = EvaluatorFiles::create(private_parent, |file| {
        file.write_all(ir)?;
        Ok(StagedEvaluatorIr { bytes: ir.len(), blake3: blake3::hash(ir) })
    })?;
    write_private_manifest(file.create_manifest()?, file.module_path())?;
    wosy_project::packages::execute_observed_comptime_set(project, scope, expected_config, attempt, watches,
        &evaluator_stages(std::slice::from_ref(&file), max_module_bytes, wosy_compiler::OrderedClosedWasip1::MAX_STDOUT),
        deadline, cancelled)
}

impl<'plan> StagedPendingPairWasip1<'plan> {
    pub(crate) fn create(
        observation: PendingPairComptimeInputObservation, plan: &'plan BoundedPairStartupWasip1Output,
        execution_seal: PreparedComptimeExecutionSeal, private_parent: &Path, max_ir_bytes: usize,
    ) -> io::Result<Self> {
        require_no_parameters(!observation.declarations().parameters().is_empty())?;
        let files = stage_pair_evaluators(plan, private_parent, max_ir_bytes)?;
        Ok(Self { observation, plan, files, execution_seal })
    }

    pub fn observation(&self) -> &PendingPairComptimeInputObservation { &self.observation }
    pub fn plan(&self) -> &BoundedPairStartupWasip1Output { self.plan }
    pub(crate) fn execution_seal(&self) -> &PreparedComptimeExecutionSeal { &self.execution_seal }

    /// Inspect one already captured terminal per distinct staged evaluator,
    /// in evaluator-index order. Bytes/status are not launch provenance or
    /// permission to apply a result. A failed terminal yields no readback.
    pub fn read_stdout(&self, terminals: &[(ExitStatus, &[u8])])
        -> Result<PendingPairWasip1Readback<'_>, PendingPairWasip1ReadbackError>
    {
        let observed = self.plan.observe_stdout(
            terminals,
            |index, (status, bytes)| {
                after_success(*status, || Ok(*bytes))
                    .map_err(|error| PendingPairWasip1ReadbackError::Evaluator { index, error })
            },
            |expected, supplied| PendingPairWasip1ReadbackError::TerminalCount { expected, supplied },
            |index, error| PendingPairWasip1ReadbackError::Evaluator { index, error: ComptimeReadbackError::Output(error) },
        )?;
        Ok(PendingPairWasip1Readback { observation: &self.observation, observed })
    }

    pub(crate) fn execution_stages(&self, max_module_bytes: u64) -> Vec<ComptimeEvaluatorStage<'_>> {
        evaluator_stages(&self.files, max_module_bytes, self.plan.max_stdout())
    }

    pub fn evaluators(&self) -> impl ExactSizeIterator<Item = StagedPendingPairWasip1Evaluator<'_>> {
        let project = Path::new(&self.observation.binding().selected.project);
        let tools = self.observation.declarations().tools();
        self.files.iter().zip(self.plan.evaluators()).map(move |(files, evaluator)| {
            StagedPendingPairWasip1Evaluator { project, tools, files, evaluator }
        })
    }
}

#[cfg(test)]
fn tool_invocation(invocation: ClosedComptimeInvocation<'_>) -> wosy_project::packages::ComptimeToolInvocation<'_> {
    wosy_project::packages::ComptimeToolInvocation {
        executable: invocation.executable, working_directory: invocation.working_directory,
        arguments: invocation.arguments,
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
    observation: &'stage PendingPairComptimeInputObservation,
    observed: ObservedPairStartupWasip1<'stage>,
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
    /// Exact staged source/site scope for these (possibly untrusted)
    /// terminal observations. This does not issue a result root.
    pub fn observation(&self) -> &PendingPairComptimeInputObservation { self.observation }

    /// Typed results in distinct evaluator order. This representation is
    /// observation only; callers cannot use it as a launch/commit receipt.
    pub fn scalar_results(&self) -> Option<Vec<wosy_protocol::ComptimeScalarValue>> {
        use wosy_protocol::ComptimeScalarValue as Value;
        self.observed.evaluator_values().map(|(ty, value)| match (ty, value) {
            (ScalarType::Bool, ScalarExpression::Boolean { value, .. }) => Some(Value::Bool(*value)),
            (ScalarType::I32, ScalarExpression::Integer { value, .. }) => {
                Some(Value::Signed { width: 32, value: i32::try_from(value).ok()?.into() })
            }
            (ScalarType::U32, ScalarExpression::Integer { value, .. }) => {
                Some(Value::Unsigned { width: 32, value: u32::try_from(value).ok()?.into() })
            }
            _ => None,
        }).collect()
    }

    pub fn occurrences(&self) -> impl ExactSizeIterator<Item = PendingPairWasip1Occurrence<'_>> {
        self.observed.occurrences().map(|(case, value)| occurrence(case, value))
    }

    /// The original initializer, not a replacement CST or applied value.
    pub fn initializers(&self) -> impl ExactSizeIterator<Item = (wosy_syntax::CstNode, PendingPairWasip1Occurrence<'_>)> {
        self.observed.initializers().map(|(node, case, value)| (node, occurrence(case, value)))
    }
}

fn occurrence<'a>(case: PairStartupOccurrence<'a>, value: Option<&'a ScalarExpression>) -> PendingPairWasip1Occurrence<'a> {
    match case {
        PairStartupOccurrence::Specialization(call, path) => PendingPairWasip1Occurrence::Specialization {
            call, path, value: value.expect("compiler associates a value with each reached evaluator"),
        },
        PairStartupOccurrence::Bypass(bypass) => PendingPairWasip1Occurrence::Bypass(bypass),
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

#[cfg(test)]
mod process_tests {
    use super::*;
    use std::{ffi::OsStr, time::{Duration, Instant}};

    fn owned_process(
        invocation: ClosedComptimeInvocation<'_>, max_stdout: Option<usize>,
        deadline: Instant, cancelled: &impl Fn() -> bool,
    ) -> io::Result<wosy_project::packages::OwnedComptimeToolResult> {
        wosy_project::packages::execute_selected_comptime_tool(tool_invocation(invocation), max_stdout, deadline, cancelled)
    }

    fn shell(code: &str) -> ClosedComptimeInvocation<'_> {
        ClosedComptimeInvocation {
            executable: OsStr::new("/bin/sh"), working_directory: Path::new("/"),
            arguments: vec![OsStr::new("-c"), OsStr::new(code)],
        }
    }

    #[test]
    fn owned_runner_retains_exact_stdout_or_rejects_the_terminal() {
        let deadline = || Instant::now() + Duration::from_secs(3);
        let ordinary = owned_process(shell("printf '42\\n'"), Some(3), deadline(), &|| false).unwrap();
        assert!(ordinary.status().success());
        assert_eq!(ordinary.stdout(), b"42\n");
        assert!(owned_process(shell("printf '42\\n'"), Some(2), deadline(), &|| false)
            .err().unwrap().to_string().contains("stdout bound"));
        let ordinary = owned_process(shell("exit 7"), None, deadline(), &|| false).unwrap();
        assert_eq!(ordinary.status().code(), Some(7));
    }

    #[test]
    fn running_selected_process_obeys_cancellation_and_deadline() {
        let deadline = Instant::now() + Duration::from_millis(50);
        assert_eq!(owned_process(shell("while :; do :; done"), None, deadline, &|| false)
            .err().unwrap().kind(), io::ErrorKind::TimedOut);
        let cancel_at = Instant::now() + Duration::from_millis(50);
        assert_eq!(owned_process(shell("while :; do :; done"), Some(8), cancel_at + Duration::from_secs(3),
            &|| Instant::now() >= cancel_at).err().unwrap().kind(), io::ErrorKind::Interrupted);
    }
}
