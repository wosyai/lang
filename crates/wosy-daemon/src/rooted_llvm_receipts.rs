//! Positive, process-local executable LLVM receipts. Never an artifact key.
use super::project_declarations::RootedOrder;
use std::{collections::VecDeque, sync::{Arc, Mutex, MutexGuard}};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use wosy_protocol::NodeId;

const MAX_ENTRIES: usize = 4;
const MAX_BYTES: usize = 4 << 20;
const MAX_FLIGHT_RESULTS: usize = 2;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct ExecutableLlvmKey(NodeId);

impl ExecutableLlvmKey {
    pub fn new(
        image: NodeId, config: NodeId, executable: NodeId, dependencies: NodeId,
        profile: &str, order: &RootedOrder,
    ) -> Self {
        let mut hash = blake3::Hasher::new_derive_key("wosy rooted executable LLVM receipt v1");
        for id in [image, config, executable, dependencies] { hash.update(&id.0); }
        field(&mut hash, profile.as_bytes());
        hash.update(&(order.modules.len() as u64).to_be_bytes());
        // Bind the exact compiler owners and resolved destinations consumed
        // by the emitter, not any authored metadata or detached file order.
        for module in &order.modules {
            let source = &module.file.metadata.source;
            field(&mut hash, source.project.as_bytes());
            field(&mut hash, source.package.as_bytes());
            field(&mut hash, source.path.as_path().as_os_str().as_encoded_bytes());
            hash.update(&module.file.executable.0);
            hash.update(&(module.imports.len() as u64).to_be_bytes());
            for destination in &module.imports { hash.update(&(destination.0 as u64).to_be_bytes()); }
        }
        Self(NodeId(*hash.finalize().as_bytes()))
    }
}

fn field(hash: &mut blake3::Hasher, value: &[u8]) {
    hash.update(&(value.len() as u64).to_be_bytes()).update(value);
}

pub(super) struct ExecutableLlvmReceipt {
    pub digest: NodeId,
    pub bytes: Box<[u8]>,
}

pub(super) struct ExecutableLlvmReceipts {
    memo: Mutex<Receipts>,
    flight_space: Arc<Semaphore>,
}

impl Default for ExecutableLlvmReceipts {
    fn default() -> Self {
        Self { memo: Default::default(), flight_space: Arc::new(Semaphore::new(MAX_FLIGHT_RESULTS)) }
    }
}

/// One bounded reservation for an eligible immutable positive result until
/// the last already registered waiter has taken or abandoned it. This caps
/// flight-owned payload at MAX_FLIGHT_RESULTS * MAX_BYTES, not caller output.
pub(super) struct ExecutableLlvmFlight {
    _reservation: OwnedSemaphorePermit,
    pub state: FlightState,
}

pub(super) enum FlightState {
    Producing,
    Complete(Arc<ExecutableLlvmReceipt>),
    Bypass,
}

pub(super) struct StagedExecutableLlvmReceipt(ExecutableLlvmKey, Arc<ExecutableLlvmReceipt>);

impl StagedExecutableLlvmReceipt {
    pub fn new(key: ExecutableLlvmKey, digest: NodeId, bytes: &[u8]) -> Option<Self> {
        (bytes.len() <= MAX_BYTES).then(|| Self(key, Arc::new(ExecutableLlvmReceipt { digest, bytes: bytes.into() })))
    }
}

pub(super) struct ExecutableLlvmReceiptAdmission<'a>(MutexGuard<'a, Receipts>);

#[derive(Default)]
struct Receipts {
    fifo: VecDeque<(ExecutableLlvmKey, Arc<ExecutableLlvmReceipt>)>,
    bytes: usize,
}

impl ExecutableLlvmReceipts {
    pub fn get(&self, key: ExecutableLlvmKey) -> Option<Arc<ExecutableLlvmReceipt>> {
        let receipts = self.memo.lock().expect("executable LLVM receipt memo");
        receipts.fifo.iter().find(|(stored, _)| *stored == key).map(|(_, receipt)| Arc::clone(receipt))
    }

    pub fn admission(&self) -> ExecutableLlvmReceiptAdmission<'_> {
        ExecutableLlvmReceiptAdmission(self.memo.lock().expect("executable LLVM receipt memo"))
    }

    pub fn reserve_flight(&self) -> Option<ExecutableLlvmFlight> {
        let reservation = Arc::clone(&self.flight_space).try_acquire_owned().ok()?;
        Some(ExecutableLlvmFlight { _reservation: reservation, state: FlightState::Producing })
    }
}

impl ExecutableLlvmReceiptAdmission<'_> {
    /// Return the memo's exact accepted Arc for an eligible flight and any
    /// retired payloads to be freed after leaving all gates.
    pub fn accept(&mut self, staged: StagedExecutableLlvmReceipt) -> (
        Option<Arc<ExecutableLlvmReceipt>>, Vec<Arc<ExecutableLlvmReceipt>>,
    ) {
        let StagedExecutableLlvmReceipt(key, receipt) = staged;
        let memo = &mut self.0;
        if let Some((_, same)) = memo.fifo.iter().find(|(stored, _)| *stored == key) {
            return ((same.digest == receipt.digest).then(|| Arc::clone(same)), vec![receipt]);
        }
        let mut retired = Vec::new();
        let bytes = receipt.bytes.len();
        while memo.fifo.len() == MAX_ENTRIES || memo.bytes > MAX_BYTES - bytes {
            let (_, oldest) = memo.fifo.pop_front().expect("receipt retention exceeds its bound");
            memo.bytes -= oldest.bytes.len();
            retired.push(oldest);
        }
        memo.bytes += bytes;
        memo.fifo.push_back((key, Arc::clone(&receipt)));
        (Some(receipt), retired)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_capacity_counts_keys_and_retained_payload() {
        let memo = ExecutableLlvmReceipts::default();
        let key = |n| ExecutableLlvmKey(NodeId([n; 32]));
        let digest = NodeId([17; 32]);
        let large = vec![3; MAX_BYTES / 2];
        let accept = |id, digest, bytes: &[u8]| {
            let staged = StagedExecutableLlvmReceipt::new(key(id), digest, bytes).unwrap();
            drop(memo.admission().accept(staged));
        };
        accept(1, digest, &large);
        accept(2, digest, &large);
        assert_eq!(memo.get(key(1)).unwrap().bytes.len(), large.len());
        accept(3, digest, &[1]);
        assert!(memo.get(key(1)).is_none());
        assert!(memo.get(key(2)).is_some());
        for n in 4..=6 { accept(n, digest, &[1]); }
        assert!(memo.get(key(2)).is_none());
        accept(3, NodeId([8; 32]), &[2]);
        assert_eq!(memo.get(key(3)).unwrap().digest, digest);
        assert_eq!(memo.memo.lock().unwrap().bytes, MAX_ENTRIES);
        assert!(StagedExecutableLlvmReceipt::new(key(7), digest, &vec![0; MAX_BYTES + 1]).is_none());
    }

    #[test]
    fn flight_result_reservations_are_bounded_and_reusable() {
        let memo = ExecutableLlvmReceipts::default();
        let mut flights: Vec<_> = (0..MAX_FLIGHT_RESULTS).map(|_| memo.reserve_flight().unwrap()).collect();
        assert!(memo.reserve_flight().is_none());
        drop(flights.pop());
        assert!(memo.reserve_flight().is_some());
    }
}
