//! One native connection to the existing fixed-scope source-only dispatcher.
use super::{BuildSessionEnd, Result, SourceStorageWorker};
use std::{path::Path, time::Duration};
use tokio::time::{sleep_until, Instant};
use wosy_protocol::{native_diagnostics::NativeDiagnosticListener, BuildSourceSelection};

/// The local activator nominates one already acknowledged source. No wire
/// request can choose the workspace, target, root or replay content.
pub struct NativeKnownDiagnostics<'worker> {
    listener: NativeDiagnosticListener,
    worker: &'worker SourceStorageWorker,
    selected: BuildSourceSelection,
}

impl SourceStorageWorker {
    /// Explicit Unix/Tokio opt-in. The caller owns activation and knows the
    /// acknowledged scope; this does not inspect client SQLite or start a
    /// global daemon. A Tokio I/O and time runtime is required.
    pub fn listen_native_known_diagnostics(
        &self, private_directory: &Path, selected: BuildSourceSelection,
    ) -> Result<NativeKnownDiagnostics<'_>> {
        let listener = NativeDiagnosticListener::bind_new(private_directory)?;
        Ok(NativeKnownDiagnostics { listener, worker: self, selected })
    }
}

impl NativeKnownDiagnostics<'_> {
    /// Accept at most one same-account connection and use one outer
    /// lifetime for accept and the existing session. Missing source produces
    /// that session's terminal replay-required response; no replay is read.
    pub async fn serve(self, lifetime: Duration) -> Result<BuildSessionEnd> {
        let deadline = Instant::now().checked_add(lifetime).ok_or("local diagnostic deadline is out of range")?;
        if Instant::now() >= deadline { return Ok(BuildSessionEnd::Expired); }
        let stream = tokio::select! {
            biased;
            _ = sleep_until(deadline) => return Ok(BuildSessionEnd::Expired),
            stream = self.listener.accept() => stream,
        };
        if Instant::now() >= deadline { return Ok(BuildSessionEnd::Expired); }
        self.worker.serve_known_build_session_before(stream?, self.selected, lifetime, deadline.into_std()).await
    }
}

/// An independently activated, one-connection projection outlet. There
/// is no replay, builder, artifact write, or broader diagnostic authority.
pub struct NativeKnownExecutableInputs<'worker> {
    native: NativeKnownDiagnostics<'worker>,
    profile: String,
    limits: super::BaseCstAdmissionLimits,
    max_partition_bytes: usize,
    activator: u32,
}

impl SourceStorageWorker {
    pub fn listen_native_known_executable_inputs(
        &self, private_directory: &Path, selected: BuildSourceSelection, profile: String,
        limits: super::BaseCstAdmissionLimits, max_partition_bytes: usize, activator: u32,
    ) -> Result<NativeKnownExecutableInputs<'_>> {
        if profile.is_empty() || profile.len() > wosy_protocol::MAX_ORDINARY_LLVM_PROFILE_BYTES {
            return Err("invalid native executable projection profile".into());
        }
        let native = self.listen_native_known_diagnostics(private_directory, selected)?;
        Ok(NativeKnownExecutableInputs { native, profile, limits, max_partition_bytes, activator })
    }
}

impl NativeKnownExecutableInputs<'_> {
    pub async fn serve(self, lifetime: Duration) -> Result<BuildSessionEnd> {
        let deadline = Instant::now().checked_add(lifetime).ok_or("local projection deadline is out of range")?;
        let stream = tokio::select! {
            biased;
            _ = sleep_until(deadline) => return Ok(BuildSessionEnd::Expired),
            stream = self.native.listener.accept_direct_parent(self.activator) => stream?,
        };
        if Instant::now() >= deadline { return Ok(BuildSessionEnd::Expired); }
        self.native.worker.serve_native_executable_inputs_before(stream, self.native.selected, self.profile,
            self.limits, self.max_partition_bytes, lifetime, deadline.into_std()).await
    }
}
