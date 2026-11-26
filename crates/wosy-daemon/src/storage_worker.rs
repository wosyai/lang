//! Opt-in blocking I/O admission for the rooted source inventory.
use super::{retention_plan::SourceRead, Result, SourceRetentionInventory, SourceStore};
use std::future::Future;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::{runtime::Handle, sync::{watch, Mutex, MutexGuard, OwnedMutexGuard, OwnedSemaphorePermit, Semaphore}, task::JoinHandle};
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
    #[cfg(test)]
    after_cancellable_phase: Option<Arc<dyn Fn() + Send + Sync>>,
    #[cfg(test)]
    on_gate_wait: Option<Arc<dyn Fn() + Send + Sync>>,
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
            #[cfg(test)]
            after_cancellable_phase: None,
            #[cfg(test)]
            on_gate_wait: None,
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

    pub(super) fn retained_store(&self) -> Arc<SourceStore> { Arc::clone(&self.store) }

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
        &self, slot: OwnedSemaphorePermit, cancellation: Option<QueryCancellation>,
        query: impl FnOnce(QueryWorker) -> F + Send + 'static,
    ) -> JoinHandle<T>
    where T: Send + 'static, F: Future<Output = T> + Send + 'static {
        let worker = QueryWorker {
            store: Arc::clone(&self.store), runtime: self.runtime.clone(), slot: Arc::new(slot),
            #[cfg(test)]
            after_phase: cancellation.as_ref().and(self.after_cancellable_phase.as_ref()).cloned(),
            #[cfg(test)]
            on_gate_wait: cancellation.as_ref().and(self.on_gate_wait.as_ref()).cloned(),
            cancellation,
        };
        self.runtime.spawn(query(worker))
    }

    #[cfg(test)]
    pub(super) fn after_cancellable_phase(&mut self, hook: impl Fn() + Send + Sync + 'static) {
        self.after_cancellable_phase = Some(Arc::new(hook));
    }

    #[cfg(test)]
    pub(super) fn on_cancellable_gate_wait(&mut self, hook: impl Fn() + Send + Sync + 'static) {
        self.on_gate_wait = Some(Arc::new(hook));
    }
}

/// Only the framed request owns the sender; it sends no values. Dropping
/// it both signals blocking phase boundaries and wakes async gate waiters.
#[derive(Clone)]
pub(super) struct QueryCancellation {
    stopped: watch::Receiver<()>,
    cutoff: Option<Instant>,
}

impl QueryCancellation {
    pub fn observe() -> (watch::Sender<()>, Self) { Self::observe_before(None) }

    pub fn observe_before(cutoff: Option<Instant>) -> (watch::Sender<()>, Self) {
        let (observer, stopped) = watch::channel(());
        (observer, Self { stopped, cutoff })
    }

    pub fn check_cutoff(&self) -> Result<()> {
        if self.cutoff.is_some_and(|time| Instant::now() >= time) {
            Err(super::build_session::BuildRequestRejection::Expired.into())
        } else {
            Ok(())
        }
    }

    fn stopped() -> super::Error { "framed Build analysis no longer has an observer".into() }

    fn check(cancellation: Option<&Self>) -> Result<()> {
        if let Some(signal) = cancellation {
            signal.check_cutoff()?;
            if signal.stopped.has_changed().is_err() { return Err(Self::stopped()); }
        }
        Ok(())
    }
}

/// One admitted query. Blocking phases never await a commit, and keep the
/// same slot and admitted source alive even if the awaiting task is dropped.
pub(super) struct QueryWorker {
    pub store: Arc<SourceStore>,
    runtime: Handle,
    slot: Arc<OwnedSemaphorePermit>,
    cancellation: Option<QueryCancellation>,
    #[cfg(test)]
    after_phase: Option<Arc<dyn Fn() + Send + Sync>>,
    #[cfg(test)]
    on_gate_wait: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl QueryWorker {
    pub(super) fn check_active(&self) -> Result<()> {
        QueryCancellation::check(self.cancellation.as_ref())
    }

    pub async fn phase<T: Send + 'static>(
        &self, source: Option<&SourceRead>, job: impl FnOnce(&SourceStore) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        self.check_active()?;
        let store = Arc::clone(&self.store);
        let ownership = (Arc::clone(&self.slot), source.cloned());
        let cancellation = self.cancellation.clone();
        #[cfg(test)]
        let after_phase = self.after_phase.clone();
        let result = self.runtime.spawn_blocking(move || {
            let _ownership = ownership;
            QueryCancellation::check(cancellation.as_ref())?;
            let result = job(&store);
            #[cfg(test)]
            if let Some(hook) = after_phase { hook(); }
            result
        }).await?;
        // Consume a phase's output only while it has an observer. In
        // particular, do not hand off a newly staged transaction or perform
        // another compiler phase after a disconnected running read drains.
        QueryCancellation::check(self.cancellation.as_ref())?;
        result
    }
}

#[derive(Clone, Copy)]
pub(super) struct QueryPhases<'a> {
    store: &'a SourceStore,
    worker: Option<&'a QueryWorker>,
    source: Option<&'a SourceRead>,
}

impl<'a> QueryPhases<'a> {
    pub fn new(store: &'a SourceStore, worker: Option<&'a QueryWorker>, source: Option<&'a SourceRead>) -> Self {
        Self { store, worker, source }
    }

    pub fn direct(store: &'a SourceStore) -> Self {
        Self::new(store, None, None)
    }

    pub async fn lock<T>(self, gate: &Mutex<T>) -> Result<MutexGuard<'_, T>> {
        self.wait_for_gate(gate.lock()).await
    }

    pub async fn lock_owned<T>(self, gate: Arc<Mutex<T>>) -> Result<OwnedMutexGuard<T>> {
        self.wait_for_gate(gate.lock_owned()).await
    }

    async fn wait_for_gate<T>(self, acquire: impl Future<Output = T>) -> Result<T> {
        let Some(worker) = self.worker else { return Ok(acquire.await); };
        let Some(signal) = &worker.cancellation else { return Ok(acquire.await); };
        #[cfg(test)]
        let acquire = async {
            let mut acquire = std::pin::pin!(acquire);
            let mut hook = worker.on_gate_wait.as_ref();
            std::future::poll_fn(|cx| {
                let result = acquire.as_mut().poll(cx);
                if result.is_pending() {
                    if let Some(hook) = hook.take() { hook(); }
                }
                result
            }).await
        };
        // Select only over acquiring the existing FIFO gate, never work
        // performed under its guard or a handed-off storage transaction.
        let mut stopped = signal.stopped.clone();
        signal.check_cutoff()?;
        let cutoff = async {
            match signal.cutoff {
                Some(time) => tokio::time::sleep_until(tokio::time::Instant::from_std(time)).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            biased;
            _ = cutoff => Err(super::build_session::BuildRequestRejection::Expired.into()),
            _ = stopped.changed() => Err(QueryCancellation::stopped()),
            guard = acquire => {
                signal.check_cutoff()?;
                Ok(guard)
            },
        }
    }

    /// Check the optional NativeKnown cutoff before starting an async commit.
    /// A transaction that already started drains under its existing rules.
    pub fn check_cutoff(self) -> Result<()> {
        match self.worker.and_then(|worker| worker.cancellation.as_ref()) {
            Some(signal) => signal.check_cutoff(),
            None => Ok(()),
        }
    }

    pub async fn run<T: Send + 'static>(
        self, job: impl FnOnce(&SourceStore) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        match self.worker {
            Some(worker) => worker.phase(self.source, job).await,
            None => job(self.store),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::mpsc;

    #[test]
    fn native_work_cutoff_checks_the_clock_with_a_live_observer() {
        let (_observer, signal) = QueryCancellation::observe_before(Some(Instant::now()));
        let rejected = QueryCancellation::check(Some(&signal)).unwrap_err();
        assert!(matches!(rejected.downcast_ref::<crate::build_session::BuildRequestRejection>(),
            Some(crate::build_session::BuildRequestRejection::Expired)));
        let (_observer, ordinary) = QueryCancellation::observe();
        QueryCancellation::check(Some(&ordinary)).unwrap();
    }

    #[test]
    #[ignore = "requires a disposable store with two admitted source fixture roots"]
    fn detached_phase_retains_original_slot_and_root() {
        let path = PathBuf::from(std::env::var_os("WOSY_TEST_IMAGE_STORE").unwrap());
        let source = NodeId::parse_hex(&std::env::var("WOSY_TEST_IMAGE_ROOT").unwrap()).unwrap();
        let scope = NodeId::parse_hex(&std::env::var("WOSY_TEST_IMAGE_OTHER_ROOT").unwrap()).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread().max_blocking_threads(1).build().unwrap();
        runtime.block_on(async {
            let store = Arc::new(SourceStore::open(&path, 8 << 20).unwrap());
            let bound = |value| NonZeroUsize::new(value).unwrap();
            let worker = SourceStorageWorker::new(
                Arc::clone(&store), Handle::current(), bound(1), bound(1), bound(20_000),
            ).unwrap();
            let (entered, waiting) = tokio::sync::oneshot::channel();
            let (release, blocked) = mpsc::channel();
            let job = worker.spawn_reserved_query(worker.try_slot().unwrap(), None, move |worker| async move {
                let read = worker.phase(None, move |store| {
                    let (tx, read) = store.begin_source_read(source)?.ok_or("expected admitted source")?;
                    drop(tx);
                    Ok(read)
                }).await?;
                worker.phase(Some(&read), move |store| {
                    entered.send(()).map_err(|_| "fixture observer missing")?;
                    blocked.recv()?;
                    assert!(store.snapshot(source)?.is_some());
                    Ok(())
                }).await
            });
            waiting.await.unwrap();
            job.abort();
            assert!(job.await.unwrap_err().is_cancelled());
            assert!(matches!(worker.try_inspect(scope, &[source], &[]), Err(StorageAdmissionError::Busy)));
            let held = store.plan_source_retention(scope, &[source], &[], 20_000).unwrap();
            assert_eq!((held.active_read_roots, held.candidate_roots), (1, 0));
            release.send(()).unwrap();
            let done = loop {
                match worker.try_inspect(scope, &[source], &[]) {
                    Ok(census) => break census.finish().await.unwrap().inventory.plan,
                    Err(StorageAdmissionError::Busy) => tokio::task::yield_now().await,
                    Err(error) => panic!("unexpected worker refusal: {error}"),
                }
            };
            assert_eq!((done.active_read_roots, done.candidate_roots), (0, 1));
            drop(worker);
            Arc::try_unwrap(store).ok().unwrap().close().await.unwrap();
        });
    }
}
