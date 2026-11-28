use super::{require_diagnostic_time, LiveCaptureRefusal, Result, SqliteSourceState};
use crate::source_state::AcknowledgedSourceGeneration;
use std::{sync::Arc, time::Duration};
use tokio::time::Instant;
use wosy_daemon::{MainFlowInputPreparation, MainFlowInputRefusal, MainFlowSourceInput, SourceStore};

pub enum SelectedMainFlowPreparation {
    Ready(SelectedMainFlowInput),
    NotAcknowledged,
    StaleSelection,
    WorkspaceChanged,
    ProfileNotSelected,
    Refused(MainFlowInputRefusal),
}

impl From<LiveCaptureRefusal> for SelectedMainFlowPreparation {
    fn from(value: LiveCaptureRefusal) -> Self {
        match value {
            LiveCaptureRefusal::NotAcknowledged => Self::NotAcknowledged,
            LiveCaptureRefusal::StaleSelection => Self::StaleSelection,
            LiveCaptureRefusal::WorkspaceChanged => Self::WorkspaceChanged,
            LiveCaptureRefusal::ProfileNotSelected => Self::ProfileNotSelected,
        }
    }
}

/// One observed delivery owner for immutable checker inputs. It is possible
/// to reuse the exact prepared bytes with a new owner after A→B→A. A report
/// from the original A must still fail its own delivery freshness check.
pub struct SelectedMainFlowInput {
    source: AcknowledgedSourceGeneration,
    input: Arc<MainFlowSourceInput>,
    pub reused_input: bool,
    pub daemon_input_elapsed: Duration,
}

impl SelectedMainFlowInput {
    pub fn source(&self) -> &AcknowledgedSourceGeneration { &self.source }
    pub fn input(&self) -> &MainFlowSourceInput { &self.input }

    /// A consumer must check again after doing independent checker work and
    /// before handing anything back. This is a freshness observation, not
    /// authority to write files or a lock against subsequent workspace edits.
    pub fn is_current_live(
        &self, client: &mut SqliteSourceState, deadline: Instant, max_authored_source_bytes: usize,
    ) -> Result<bool> {
        Ok(reobserve(client, &self.source, self.input.profile(), deadline, max_authored_source_bytes)?.is_none())
    }
}

fn reobserve(
    client: &mut SqliteSourceState, source: &AcknowledgedSourceGeneration, profile: &str,
    deadline: Instant, max_authored_source_bytes: usize,
) -> Result<Option<LiveCaptureRefusal>> {
    if !source.is_current(client)? { return Ok(Some(LiveCaptureRefusal::StaleSelection)); }
    let current = client.capture_live(&source.selection().target, profile, deadline, max_authored_source_bytes, false)?;
    match current {
        Ok(current) if current.captured.selection() == source.selection()
            && current.captured.source.generation() == source.generation() => Ok(None),
        Ok(_) => Ok(Some(LiveCaptureRefusal::StaleSelection)),
        Err(reason) => Ok(Some(reason)),
    }
}

impl SqliteSourceState {
    /// Local adapter only. No command, socket, result key, effect publication
    /// or cache of checker outcomes is introduced by this source handoff.
    #[doc(hidden)]
    pub fn prepare_selected_main_flow_source(
        &mut self, store: &SourceStore, target: &str, profile: &str, deadline: Instant,
        max_authored_source_bytes: usize, prior: Option<&SelectedMainFlowInput>,
    ) -> Result<SelectedMainFlowPreparation> {
        use SelectedMainFlowPreparation as Outcome;
        let captured = match self.capture_live(target, profile, deadline, max_authored_source_bytes, false)? {
            Ok(live) => live.captured,
            Err(reason) => return Ok(reason.into()),
        };
        let prior = prior.map(|prepared| &prepared.input);
        let start = Instant::now();
        let prepared = store.prepare_common_main_flow_source(captured.selection(), profile, prior)?;
        let daemon_input_elapsed = start.elapsed();
        require_diagnostic_time(deadline)?;

        if let Some(reason) = reobserve(self, &captured.source, profile, deadline, max_authored_source_bytes)? {
            return Ok(reason.into());
        }
        match prepared {
            MainFlowInputPreparation::Ready(input) => {
                let owner = SelectedMainFlowInput { source: captured.source.clone(),
                    reused_input: prior.is_some_and(|prior| Arc::ptr_eq(prior, &input)),
                    daemon_input_elapsed, input };
                Ok(Outcome::Ready(owner))
            }
            MainFlowInputPreparation::Refused(reason) => Ok(Outcome::Refused(reason)),
        }
    }
}
