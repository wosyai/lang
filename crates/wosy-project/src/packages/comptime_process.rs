//! Client-side direct tool custody. Owning a terminal does not establish
//! hidden environment inputs, or admit anything to a compiler snapshot.

use std::{
    ffi::{OsStr, OsString}, io::{self, Read, Write}, os::fd::AsRawFd, path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio}, thread::sleep, time::{Duration, Instant},
};

pub struct ComptimeToolInvocation<'a> {
    pub executable: &'a OsStr,
    pub working_directory: &'a Path,
    pub arguments: Vec<&'a OsStr>,
}

/// Cannot be constructed from asserted terminal bytes or deserialized.
pub struct OwnedComptimeToolResult {
    executable: OsString,
    working_directory: PathBuf,
    arguments: Vec<OsString>,
    status: ExitStatus,
    stdout: Vec<u8>,
}

impl OwnedComptimeToolResult {
    pub fn executable(&self) -> &OsStr { &self.executable }
    pub fn working_directory(&self) -> &Path { &self.working_directory }
    pub fn arguments(&self) -> &[OsString] { &self.arguments }
    pub fn status(&self) -> ExitStatus { self.status }
    pub fn stdout(&self) -> &[u8] { &self.stdout }
}

fn command(invocation: &ComptimeToolInvocation<'_>) -> Command {
    let mut command = Command::new(invocation.executable);
    command.current_dir(invocation.working_directory).args(&invocation.arguments).stdin(Stdio::null());
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

pub fn execute_selected_comptime_tool(
    invocation: ComptimeToolInvocation<'_>, max_stdout: Option<usize>,
    deadline: Instant, cancelled: &impl Fn() -> bool,
) -> io::Result<OwnedComptimeToolResult> {
    check_tool_budget(deadline, cancelled)?;
    let mut configured = command(&invocation);
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
            if output.is_none() || !status.success() {
                return Ok(OwnedComptimeToolResult {
                    executable: invocation.executable.to_owned(),
                    working_directory: invocation.working_directory.to_owned(),
                    arguments: invocation.arguments.into_iter().map(OsStr::to_owned).collect(),
                    status, stdout: bytes,
                });
            }
        }
        sleep(deadline.saturating_duration_since(Instant::now()).min(Duration::from_millis(5)));
    }
}


/// Actual client-side selected pair, tied to one owner-held staging slot.
/// It proves the declared process calls and observations, not hidden reads.
pub struct OwnedStagedComptimePair {
    ir: blake3::Hash,
    manifest: blake3::Hash,
    module: blake3::Hash,
    builder: OwnedComptimeToolResult,
    runner: OwnedComptimeToolResult,
}

impl OwnedStagedComptimePair {
    pub fn ir(&self) -> blake3::Hash { self.ir }
    pub fn manifest(&self) -> blake3::Hash { self.manifest }
    pub fn module(&self) -> blake3::Hash { self.module }
    pub fn builder(&self) -> &OwnedComptimeToolResult { &self.builder }
    pub fn runner(&self) -> &OwnedComptimeToolResult { &self.runner }
}

#[derive(Clone, Copy)]
pub struct StagedComptimePairInputs {
    pub ir_bytes: usize,
    pub ir: blake3::Hash,
    pub max_module_bytes: u64,
    pub max_stdout: usize,
}

/// Current closed ABI: declared parameters, guest capabilities and mounts
/// are empty. The expected module name is supplied by the held scratch.
pub fn empty_wasip1_comptime_manifest(module: &Path) -> io::Result<Vec<u8>> {
    #[derive(serde::Serialize)]
    struct Manifest<'a> {
        executable: &'a Path,
        parameters: std::collections::BTreeMap<(), ()>,
        permissions: [(); 0],
        mounts: [(); 0],
    }
    serde_json::to_vec(&Manifest {
        executable: module, parameters: std::collections::BTreeMap::new(), permissions: [], mounts: [],
    }).map_err(io::Error::other)
}

impl super::ComptimeScratch {
    pub fn observe_module_digest(&self, max_bytes: u64) -> io::Result<(u64, blake3::Hash)> {
            let (file, metadata) = self.open_module()?;
            let expected = metadata.len();
            if expected == 0 || expected > max_bytes {
                return Err(io::Error::other("staged evaluator module is empty or exceeds its observation budget"));
            }
            let mut reader = file.take(expected.saturating_add(1));
            let digest = blake3::Hasher::new().update_reader(&mut reader)?.finalize();
            let end = reader.get_ref().metadata()?;
            use std::os::unix::fs::MetadataExt;
            let version = |meta: &std::fs::Metadata| {
                (meta.len(), meta.mtime(), meta.mtime_nsec(), meta.ctime(), meta.ctime_nsec(), meta.nlink())
            };
            if reader.limit() != 1 || version(&metadata) != version(&end) {
                return Err(io::Error::other("staged evaluator module changed during observation"));
            }
            Ok((expected, digest))
    }


    /// The client owns execution; this is not a daemon request handler.
    pub fn execute_staged_comptime_pair(
        &self, input: StagedComptimePairInputs, builder: ComptimeToolInvocation<'_>,
        runner: ComptimeToolInvocation<'_>, deadline: Instant, cancelled: &impl Fn() -> bool,
    ) -> io::Result<OwnedStagedComptimePair> {
        let ir = self.ir_path();
        let manifest = self.manifest_path();
        let expected_manifest = empty_wasip1_comptime_manifest(&self.module_path())?;
        if !builder.arguments.ends_with(&[ir.as_os_str(), self.output_dir().as_os_str(), manifest.as_os_str()])
            || !runner.arguments.ends_with(&[manifest.as_os_str()])
        {
            return Err(io::Error::other("selected comptime arguments do not match the held stage"));
        }
        let check_inputs = || {
            let mut bytes = Vec::new();
            self.open_ir()?.take(input.ir_bytes as u64 + 1).read_to_end(&mut bytes)?;
            if bytes.len() != input.ir_bytes || blake3::hash(&bytes) != input.ir {
                return Err(io::Error::other("staged evaluator LLVM changed"));
            }
            bytes.clear();
            self.open_manifest()?.take(expected_manifest.len() as u64 + 1).read_to_end(&mut bytes)?;
            if bytes != expected_manifest {
                return Err(io::Error::other("staged evaluator manifest changed"));
            }
            Ok(())
        };
        check_inputs()?;
        let builder = execute_selected_comptime_tool(builder, None, deadline, cancelled)?;
        if !builder.status().success() { return Err(io::Error::other("selected comptime builder failed")); }
        check_inputs()?;
        let (_, module) = self.observe_module_digest(input.max_module_bytes)?;
        let runner = execute_selected_comptime_tool(runner, Some(input.max_stdout), deadline, cancelled)?;
        if !runner.status().success() { return Err(io::Error::other("selected comptime runner failed")); }
        check_inputs()?;
        if self.observe_module_digest(input.max_module_bytes)?.1 != module {
            return Err(io::Error::other("staged comptime module changed during run"));
        }
        Ok(OwnedStagedComptimePair {
            ir: input.ir, manifest: blake3::hash(&expected_manifest), module, builder, runner,
        })
    }
}

/// A per-owner local session seal. Equality is pointer identity, never
/// a digest or reconstructed caller value. The daemon retains one copy
/// when handing a prepared plan to its client.
#[derive(Clone)]
pub struct PreparedComptimeExecutionSeal(std::sync::Arc<()>);

impl PreparedComptimeExecutionSeal {
    pub fn fresh() -> Self { Self(std::sync::Arc::new(())) }
    pub fn same_owner(&self, other: &Self) -> bool { std::sync::Arc::ptr_eq(&self.0, &other.0) }
}

/// The caller names the pending compiler proposal; daemon intake still
/// checks this binding against its own prepared, live proposal.
pub struct ComptimeExecutionAttempt {
    pub seal: PreparedComptimeExecutionSeal,
    pub root: [u8; 32],
    pub generation: i64,
    pub authored_cst: [u8; 32],
}

#[derive(Clone, Copy)]
pub struct ComptimeEvaluatorStage<'a> {
    pub scratch: &'a super::ComptimeScratch,
    pub inputs: StagedComptimePairInputs,
}

/// One client-owned pre/execute/post observation. Tool receipts and watched
/// inputs cannot be mixed between independent executions through this type.
pub struct OwnedLocalComptimeExecution {
    attempt: ComptimeExecutionAttempt,
    observed: super::OwnedLocalComptimeInputs,
    pairs: Vec<OwnedStagedComptimePair>,
}

impl OwnedLocalComptimeExecution {
    pub fn attempt(&self) -> &ComptimeExecutionAttempt { &self.attempt }
    pub fn observed(&self) -> &super::OwnedLocalComptimeInputs { &self.observed }
    pub fn pairs(&self) -> &[OwnedStagedComptimePair] { &self.pairs }
}

pub fn execute_observed_comptime_set(
    project: &Path, scope: super::LocalComptimeInputScope<'_>, expected_config: [u8; 32],
    attempt: ComptimeExecutionAttempt, staged_watches: &[super::WatchedComptimeFile],
    stages: &[ComptimeEvaluatorStage<'_>],
    deadline: Instant, cancelled: &impl Fn() -> bool,
) -> io::Result<OwnedLocalComptimeExecution> {
    execute_owned_comptime_sequence(project, scope, expected_config, attempt, staged_watches,
        deadline, cancelled, |run| {
            stages.iter().try_for_each(|stage| run(*stage))
        })
}

pub struct StreamedComptimeStageLimits<'a> {
    pub private_parent: &'a Path,
    pub aggregate_ir_bytes: usize,
    pub module_bytes: u64,
    pub stdout_bytes: usize,
}

/// Run the same selected tool sequence, but keep only the current
/// evaluator's owned scratch open. Receipts retain observations, not
/// workspace paths as a continued authority over removed modules.
pub fn execute_streamed_comptime_set<'ir>(
    project: &Path, scope: super::LocalComptimeInputScope<'_>, expected_config: [u8; 32],
    attempt: ComptimeExecutionAttempt, staged_watches: &[super::WatchedComptimeFile],
    irs: impl Iterator<Item = &'ir [u8]> + Clone, limits: StreamedComptimeStageLimits<'_>,
    deadline: Instant, cancelled: &impl Fn() -> bool,
) -> io::Result<OwnedLocalComptimeExecution> {
    // As with full staging, reject an impossible explicit IR budget
    // before any external process. No added limit on evaluator count.
    irs.clone().try_fold(limits.aggregate_ir_bytes, |remaining, ir| {
        remaining.checked_sub(ir.len()).ok_or_else(|| io::Error::other("selected comptime evaluator IR exceeds requested budget"))
    })?;
    execute_owned_comptime_sequence(project, scope, expected_config, attempt, staged_watches,
        deadline, cancelled, |run| {
            for ir in irs {
                check_tool_budget(deadline, cancelled)?;
                let scratch = super::ComptimeScratch::create(limits.private_parent)?;
                scratch.create_ir()?.write_all(ir)?;
                scratch.create_manifest()?.write_all(&empty_wasip1_comptime_manifest(&scratch.module_path())?)?;
                run(ComptimeEvaluatorStage {
                    scratch: &scratch,
                    inputs: StagedComptimePairInputs { ir_bytes: ir.len(), ir: blake3::hash(ir),
                        max_module_bytes: limits.module_bytes, max_stdout: limits.stdout_bytes },
                })?;
                // Drop before another stage can acquire disk storage or
                // descriptors, on success as well as any early return.
            }
            Ok(())
        })
}

fn execute_owned_comptime_sequence(
    project: &Path, scope: super::LocalComptimeInputScope<'_>, expected_config: [u8; 32],
    attempt: ComptimeExecutionAttempt, staged_watches: &[super::WatchedComptimeFile],
    deadline: Instant, cancelled: &impl Fn() -> bool,
    visit: impl FnOnce(&mut dyn FnMut(ComptimeEvaluatorStage<'_>) -> io::Result<()>) -> io::Result<()>,
) -> io::Result<OwnedLocalComptimeExecution> {
    fn invoke<'a, const N: usize>(
        project: &'a Path, command: &'a [String], args: &'a [String], suffix: [&'a Path; N],
    ) -> ComptimeToolInvocation<'a> {
        ComptimeToolInvocation {
            executable: OsStr::new(&command[0]),
            working_directory: project,
            arguments: command[1..].iter().chain(args).map(OsStr::new)
                .chain(suffix.map(Path::as_os_str)).collect(),
        }
    }

    check_tool_budget(deadline, cancelled)?;
    let observe = || super::observe_owned_local_comptime_inputs(project, super::LocalComptimeInputScope {
        target: scope.target, profile: scope.profile, site: scope.site, watch_limits: scope.watch_limits,
    }, expected_config).map_err(io::Error::other);
    let before = observe()?;
    if before.watches() != staged_watches {
        return Err(io::Error::other("declared comptime inputs changed after evaluator staging"));
    }
    if !before.declarations().parameters().is_empty() {
        return Err(io::Error::other("closed comptime execution requires empty declared parameters"));
    }
    let tools = before.declarations().tools();
    let (_, builder) = tools.builder();
    let (_, runner) = tools.runner();
    let mut pairs = Vec::new();
    visit(&mut |stage| {
        let ir = stage.scratch.ir_path();
        let manifest = stage.scratch.manifest_path();
        pairs.push(stage.scratch.execute_staged_comptime_pair(
            stage.inputs,
            invoke(project, &builder.command, &builder.args, [&ir, stage.scratch.output_dir(), &manifest]),
            invoke(project, &runner.command, &runner.args, [&manifest]),
            deadline, cancelled,
        )?);
        Ok(())
    })?;
    let after = observe()?;
    check_tool_budget(deadline, cancelled)?;
    if before.watches() != after.watches() {
        return Err(io::Error::other("declared comptime inputs changed during execution"));
    }
    Ok(OwnedLocalComptimeExecution { attempt, observed: after, pairs })
}
