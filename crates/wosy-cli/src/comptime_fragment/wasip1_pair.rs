//! Private local manifest for the selected WASIp1 tool pair.
//! Staging never authenticates tools, invokes them, or accepts a result.
use super::{provision::EvaluatorFiles, selected_builder_invocation, selected_runner_invocation, stdout::after_success,
    ClosedComptimeInvocation, ComptimeReadbackError, ProvisionedWasip1Comptime, StagedEvaluatorIr};
use crate::source_state::PendingPairComptimeInputObservation;
use serde::Serialize;
use std::{collections::BTreeMap, fmt, io::{self, Read, Write}, path::Path, os::fd::AsRawFd, process::{Child, Command, ExitStatus, Stdio}, thread::sleep, time::{Duration, Instant}};
use wosy_compiler::{BoundedPairStartupWasip1Output, BoundedStartupWasip1Evaluator, DirectStartupComptimeUse,
    ObservedPairStartupWasip1, PairStartupBypass, PairStartupCallPath, PairStartupOccurrence, ScalarExpression, ScalarType};
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

fn private_manifest(module: &Path) -> ResolvedPrivateManifest<'_> {
    ResolvedPrivateManifest { executable: module, parameters: BTreeMap::new(), permissions: [], mounts: [] }
}

fn write_private_manifest(file: std::fs::File, module: &Path) -> io::Result<()> {
    serde_json::to_writer(file, &private_manifest(module)).map_err(io::Error::other)
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

    pub(crate) fn invoke_selected(
        &self, max_module_bytes: u64, deadline: Instant, cancelled: impl Fn() -> bool,
    ) -> io::Result<ObservedPairProcessTerminals> {
        let mut terminals = Vec::new();
        let mut modules = Vec::new();
        for evaluator in self.evaluators() {
            let manifest = serde_json::to_vec(&private_manifest(evaluator.expected_module_path())).map_err(io::Error::other)?;
            evaluator.files.verify_inputs(&manifest)?;
            let (build, _) = owned_process(evaluator.builder(), None, deadline, &cancelled)?;
            if !build.success() { return Err(io::Error::other("selected comptime builder failed")); }
            evaluator.files.verify_inputs(&manifest)?;
            let before = evaluator.files.observe_module(max_module_bytes)?;
            let (status, stdout) = owned_process(evaluator.runner(), Some(self.plan.max_stdout()), deadline, &cancelled)?;
            if !status.success() { return Err(io::Error::other("selected comptime runner failed")); }
            evaluator.files.verify_inputs(&manifest)?;
            let after = evaluator.files.observe_module(max_module_bytes)?;
            if before.digest != after.digest { return Err(io::Error::other("staged comptime module changed during run")); }
            modules.push(before.digest);
            terminals.push((status, stdout));
        }
        Ok(ObservedPairProcessTerminals { modules, terminals })
    }

    pub fn evaluators(&self) -> impl ExactSizeIterator<Item = StagedPendingPairWasip1Evaluator<'_>> {
        let project = Path::new(&self.observation.binding().selected.project);
        let tools = self.observation.declarations().tools();
        self.files.iter().zip(self.plan.evaluators()).map(move |(files, evaluator)| {
            StagedPendingPairWasip1Evaluator { project, tools, files, evaluator }
        })
    }
}

fn command(invocation: ClosedComptimeInvocation<'_>) -> Command {
    let mut command = Command::new(invocation.executable);
    command.current_dir(invocation.working_directory).args(invocation.arguments).stdin(Stdio::null());
    command
}

struct OwnedChild(Option<Child>);

impl OwnedChild {
    fn child(&mut self) -> &mut Child { self.0.as_mut().expect("unreaped selected child") }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn check_tool_budget(deadline: Instant, cancelled: &impl Fn() -> bool) -> io::Result<()> {
    if cancelled() { return Err(io::Error::new(io::ErrorKind::Interrupted, "selected comptime tools were cancelled")); }
    if Instant::now() >= deadline {
        return Err(io::Error::new(io::ErrorKind::TimedOut, "selected comptime tool deadline expired"));
    }
    Ok(())
}

fn owned_process(
    invocation: ClosedComptimeInvocation<'_>, max_stdout: Option<usize>,
    deadline: Instant, cancelled: &impl Fn() -> bool,
) -> io::Result<(ExitStatus, Vec<u8>)> {
    check_tool_budget(deadline, cancelled)?;
    let mut configured = command(invocation);
    if max_stdout.is_some() { configured.stdout(Stdio::piped()); }
    let mut guard = OwnedChild(Some(configured.spawn()?));
    let mut output = guard.child().stdout.take();
    if let Some(output) = output.as_ref() {
        let fd = output.as_raw_fd();
        // SAFETY: the child stdout still owns this descriptor. Preserve its
        // existing flags and add nonblocking reads so tool deadlines apply.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
            return Err(io::Error::last_os_error());
        }
    }
    let mut bytes = Vec::new();
    let mut status = None;
    loop {
        check_tool_budget(deadline, cancelled)?;
        if let Some(pipe) = output.as_mut() {
            let mut chunk = [0; 4096];
            let remaining = max_stdout.expect("pipe has an output cap").saturating_sub(bytes.len());
            let to_read = remaining.saturating_add(1).min(chunk.len());
            match pipe.read(&mut chunk[..to_read]) {
                Ok(0) => output = None,
                Ok(n) if n > remaining => return Err(io::Error::other("selected comptime runner exceeded its stdout bound")),
                Ok(n) => {
                    bytes.extend_from_slice(&chunk[..n]);
                    continue;
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock || error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
        if status.is_none() {
            status = guard.child().try_wait()?;
            if status.is_some() { guard.0 = None; }
        }
        if let Some(status) = status {
            if output.is_none() || !status.success() { return Ok((status, bytes)); }
        }
        sleep(deadline.saturating_duration_since(Instant::now()).min(Duration::from_millis(5)));
    }
}

pub(crate) struct ObservedPairProcessTerminals {
    pub(crate) modules: Vec<blake3::Hash>,
    pub(crate) terminals: Vec<(ExitStatus, Vec<u8>)>,
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
    use std::ffi::OsStr;

    fn shell(code: &str) -> ClosedComptimeInvocation<'_> {
        ClosedComptimeInvocation {
            executable: OsStr::new("/bin/sh"), working_directory: Path::new("/"),
            arguments: vec![OsStr::new("-c"), OsStr::new(code)],
        }
    }

    #[test]
    fn owned_runner_retains_exact_stdout_or_rejects_the_terminal() {
        let deadline = || Instant::now() + Duration::from_secs(3);
        let (status, output) = owned_process(shell("printf '42\\n'"), Some(3), deadline(), &|| false).unwrap();
        assert!(status.success());
        assert_eq!(&output, b"42\n");
        assert!(owned_process(shell("printf '42\\n'"), Some(2), deadline(), &|| false)
            .unwrap_err().to_string().contains("stdout bound"));
        let (status, _) = owned_process(shell("exit 7"), None, deadline(), &|| false).unwrap();
        assert_eq!(status.code(), Some(7));
    }

    #[test]
    fn running_selected_process_obeys_cancellation_and_deadline() {
        let deadline = Instant::now() + Duration::from_millis(50);
        assert_eq!(owned_process(shell("while :; do :; done"), None, deadline, &|| false)
            .unwrap_err().kind(), io::ErrorKind::TimedOut);
        let cancel_at = Instant::now() + Duration::from_millis(50);
        assert_eq!(owned_process(shell("while :; do :; done"), Some(8), cancel_at + Duration::from_secs(3),
            &|| Instant::now() >= cancel_at).unwrap_err().kind(), io::ErrorKind::Interrupted);
    }
}
