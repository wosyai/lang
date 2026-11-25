//! Opt-in blocking I/O admission for the rooted source inventory.
use super::{Result, SourceRetentionInventory, SourceStore};
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::{runtime::Handle, sync::Semaphore, task::JoinHandle};
use wosy_protocol::NodeId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageAdmissionError {
    Busy,
    SelectionOverBudget,
}

impl std::fmt::Display for StorageAdmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Busy => "source inventory worker is at capacity",
            Self::SelectionOverBudget => "source inventory selection exceeds the worker budget",
        })
    }
}

impl std::error::Error for StorageAdmissionError {}

pub struct RetentionRead {
    pub inventory: SourceRetentionInventory,
    pub queued: Duration,
    pub work: Duration,
}

pub struct RetentionReadJob(JoinHandle<Result<RetentionRead>>);

impl RetentionReadJob {
    /// Dropping the observer does not release a running or queued read's slot.
    pub async fn finish(self) -> Result<RetentionRead> {
        self.0.await?
    }
}

/// Bounds one inventory service's admitted jobs and copied root IDs. Each
/// job uses the store's root/pin count and distinct-node census limit. Does
/// not bound bytes read, other work, the global blocking pool, or total RSS.
pub struct SourceStorageWorker {
    store: Arc<SourceStore>,
    runtime: Handle,
    slots: Arc<Semaphore>,
    selection_limit: usize,
    census_limit: usize,
}

impl SourceStorageWorker {
    pub fn new(
        store: Arc<SourceStore>, runtime: Handle, slots: NonZeroUsize,
        selection_limit: NonZeroUsize, census_limit: NonZeroUsize,
    ) -> Result<Self> {
        if slots.get() > Semaphore::MAX_PERMITS {
            return Err("source inventory slot count exceeds the worker limit".into());
        }
        Ok(Self {
            store, runtime, slots: Arc::new(Semaphore::new(slots.get())),
            selection_limit: selection_limit.get(), census_limit: census_limit.get(),
        })
    }

    pub fn try_inspect(
        &self, scope: NodeId, nominated: &[NodeId], keep: &[NodeId],
    ) -> std::result::Result<RetentionReadJob, StorageAdmissionError> {
        if nominated.len().saturating_add(keep.len()) > self.selection_limit {
            return Err(StorageAdmissionError::SelectionOverBudget);
        }
        let slot = Arc::clone(&self.slots).try_acquire_owned().map_err(|_| StorageAdmissionError::Busy)?;
        let store = Arc::clone(&self.store);
        let nominated = nominated.to_vec();
        let keep = keep.to_vec();
        let limit = self.census_limit;
        let submitted = Instant::now();
        let job = self.runtime.spawn_blocking(move || {
            let start = Instant::now();
            let inventory = store.inspect_source_retention(scope, &nominated, &keep, limit);
            let work = start.elapsed();
            drop((store, nominated, keep));
            drop(slot);
            Ok(RetentionRead { inventory: inventory?, queued: start.duration_since(submitted), work })
        });
        Ok(RetentionReadJob(job))
    }
}
