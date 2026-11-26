//! Opt-in blocking I/O admission for the rooted source inventory.
use super::{Result, SourceRetentionInventory, SourceStore};
use std::future::Future;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::{runtime::Handle, sync::{OwnedSemaphorePermit, Semaphore}, task::JoinHandle};
use wosy_protocol::NodeId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageAdmissionError {
    Busy,
    SelectionOverBudget,
}

impl std::fmt::Display for StorageAdmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Busy => "source storage worker is at capacity",
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

/// Admission shared by rooted storage jobs. Inventory reads also bound
/// copied root IDs and use the store's census limit. Does not bound bytes
/// read, other work, the global blocking pool, or total RSS.
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
            return Err("source storage slot count exceeds the worker limit".into());
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
        let slot = self.try_slot()?;
        let nominated = nominated.to_vec();
        let keep = keep.to_vec();
        let limit = self.census_limit;
        let submitted = Instant::now();
        let job = self.spawn_reserved(slot, move |store| {
            let start = Instant::now();
            let inventory = store.inspect_source_retention(scope, &nominated, &keep, limit)?;
            Ok(RetentionRead { inventory, queued: start.duration_since(submitted), work: start.elapsed() })
        });
        Ok(RetentionReadJob(job))
    }

    pub(super) fn try_slot(&self) -> std::result::Result<OwnedSemaphorePermit, StorageAdmissionError> {
        Arc::clone(&self.slots).try_acquire_owned().map_err(|_| StorageAdmissionError::Busy)
    }

    pub(super) fn spawn_reserved<T: Send + 'static>(
        &self, slot: OwnedSemaphorePermit, job: impl FnOnce(&SourceStore) -> T + Send + 'static,
    ) -> JoinHandle<T> {
        let store = Arc::clone(&self.store);
        self.runtime.spawn_blocking(move || {
            let result = job(&store);
            drop(store);
            drop(slot);
            result
        })
    }

    pub(super) fn spawn_reserved_query<T, F>(
        &self, slot: OwnedSemaphorePermit, query: impl FnOnce(Arc<SourceStore>, Handle) -> F + Send + 'static,
    ) -> JoinHandle<T>
    where T: Send + 'static, F: Future<Output = T> + Send + 'static {
        let store = Arc::clone(&self.store);
        let runtime = self.runtime.clone();
        self.runtime.spawn(async move {
            let result = query(store, runtime).await;
            drop(slot);
            result
        })
    }

}
