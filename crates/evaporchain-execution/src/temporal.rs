//! Temporal Smart Contract Execution Engine
//!
//! Provides entropy-driven temporal execution primitives unique to EvaporChain:
//!
//! 1. **DeferredQueue** — Time-locked transactions that auto-execute when temporal
//!    conditions (epoch ranges, energy thresholds, object evaporation, contract phases)
//!    are satisfied. Transactions are submitted now but execute in the future.
//!
//! 2. **DecayWatcherEngine** — Energy threshold monitors that fire contract callbacks
//!    when an object's energy crosses a threshold during the per-block evaporation tick.
//!
//! Both integrate with the block execution pipeline and the thermodynamic model.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use evaporchain_contracts::ContractEngine;
use evaporchain_state::db::StateDB;
use evaporchain_types::{
    AccountAddress, DeferredTx, Energy, EnergyWatcher, Epoch, ObjectId, TemporalGuard,
};
use thiserror::Error;

// ═══════════════════════════════════════════════════════════════════════════
// Errors
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Error)]
pub enum TemporalError {
    #[error("deferred tx deposit too low: need {needed}, got {got}")]
    InsufficientDeposit { needed: u64, got: u64 },
    #[error("deferred tx has no guards")]
    NoGuards,
    #[error("deferred tx has no inner transaction")]
    EmptyInnerTx,
    #[error("deferred tx expired at epoch {epoch}")]
    Expired { epoch: Epoch },
    #[error("too many deferred txs in queue (max {max})")]
    QueueFull { max: usize },
    #[error("inner tx deserialization failed: {0}")]
    InnerTxInvalid(String),
    #[error("watcher object not found: {0:?}")]
    ObjectNotFound(ObjectId),
    #[error("watcher limit reached (max {max})")]
    TooManyWatchers { max: usize },
}

// ═══════════════════════════════════════════════════════════════════════════
// Constants
// ═══════════════════════════════════════════════════════════════════════════

/// Minimum deposit for a deferred tx (covers queue storage).
const MIN_DEFERRED_DEPOSIT: u64 = 10_000;
/// Gas cost to submit a deferred tx.
pub const GAS_DEFERRED_SUBMIT: u64 = 75_000;
/// Gas cost per guard evaluation.
pub const GAS_PER_GUARD: u64 = 5_000;
/// Maximum number of deferred txs in the queue.
const MAX_QUEUE_SIZE: usize = 10_000;
/// Queue storage fee deducted from deposit on submission.
const QUEUE_STORAGE_FEE: u64 = 1_000;
/// Maximum number of active watchers.
const MAX_WATCHERS: usize = 5_000;

// ═══════════════════════════════════════════════════════════════════════════
// Deferred Queue
// ═══════════════════════════════════════════════════════════════════════════

/// A deferred transaction entry in the queue, ordered by earliest possible execution.
#[derive(Debug, Clone)]
struct DeferredEntry {
    /// The deferred transaction.
    tx: DeferredTx,
    /// Earliest epoch this could fire (from AfterEpoch guards).
    earliest_epoch: Epoch,
    /// Latest epoch before expiry (from BeforeEpoch guards, u64::MAX if none).
    expiry_epoch: Epoch,
    /// Remaining deposit after queue fee.
    remaining_deposit: u64,
    /// Unique sequence number for stable ordering.
    seq: u64,
}

impl Eq for DeferredEntry {}
impl PartialEq for DeferredEntry {
    fn eq(&self, other: &Self) -> bool {
        self.seq == other.seq
    }
}

impl Ord for DeferredEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        // Min-heap: lower earliest_epoch = higher priority.
        other
            .earliest_epoch
            .cmp(&self.earliest_epoch)
            .then_with(|| other.seq.cmp(&self.seq))
    }
}

impl PartialOrd for DeferredEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Result of processing the deferred queue for one block.
#[derive(Debug, Default)]
pub struct DeferredQueueResult {
    /// Number of deferred txs that matured and had their inner tx executed.
    pub matured: usize,
    /// Number of deferred txs that expired.
    pub expired: usize,
    /// Number of deferred txs still pending.
    pub pending: usize,
    /// Inner transaction bytes for matured txs (caller deserializes and executes).
    pub matured_txs: Vec<(AccountAddress, Vec<u8>, u64)>, // (submitter, inner_bytes, gas_limit)
    /// Refund amounts for expired txs: (address, refund_amount).
    pub refunds: Vec<(AccountAddress, u64)>,
}

/// Priority queue of deferred transactions that execute when temporal conditions are met.
pub struct DeferredQueue {
    queue: BinaryHeap<DeferredEntry>,
    next_seq: u64,
}

impl DeferredQueue {
    pub fn new() -> Self {
        Self {
            queue: BinaryHeap::new(),
            next_seq: 0,
        }
    }

    /// Submit a new deferred transaction to the queue.
    pub fn submit(&mut self, tx: DeferredTx) -> Result<u64, TemporalError> {
        if tx.guards.is_empty() {
            return Err(TemporalError::NoGuards);
        }
        if tx.inner_tx_bytes.is_empty() {
            return Err(TemporalError::EmptyInnerTx);
        }
        if tx.deposit < MIN_DEFERRED_DEPOSIT {
            return Err(TemporalError::InsufficientDeposit {
                needed: MIN_DEFERRED_DEPOSIT,
                got: tx.deposit,
            });
        }
        if self.queue.len() >= MAX_QUEUE_SIZE {
            return Err(TemporalError::QueueFull {
                max: MAX_QUEUE_SIZE,
            });
        }

        // Extract scheduling hints from guards.
        let mut earliest = 0u64;
        let mut expiry = u64::MAX;
        for guard in &tx.guards {
            match guard {
                TemporalGuard::AfterEpoch(e) if *e > earliest => {
                    earliest = *e;
                }
                TemporalGuard::BeforeEpoch(e) if *e < expiry => {
                    expiry = *e;
                }
                _ => {}
            }
        }

        let remaining = tx.deposit.saturating_sub(QUEUE_STORAGE_FEE);
        let seq = self.next_seq;
        self.next_seq += 1;

        self.queue.push(DeferredEntry {
            tx,
            earliest_epoch: earliest,
            expiry_epoch: expiry,
            remaining_deposit: remaining,
            seq,
        });

        Ok(seq)
    }

    /// Process the queue for the current epoch.
    /// Returns matured inner txs for execution and expired refunds.
    pub fn process_epoch<DB: StateDB + ?Sized>(
        &mut self,
        epoch: Epoch,
        db: &DB,
        contract_engine: &ContractEngine,
    ) -> DeferredQueueResult {
        let mut result = DeferredQueueResult::default();
        let mut remaining = BinaryHeap::new();

        while let Some(entry) = self.queue.pop() {
            // Check expiry first.
            if epoch >= entry.expiry_epoch {
                result.expired += 1;
                result
                    .refunds
                    .push((entry.tx.submitter, entry.remaining_deposit));
                continue;
            }

            // Skip if not yet eligible.
            if epoch < entry.earliest_epoch {
                remaining.push(entry);
                continue;
            }

            // Evaluate all guards.
            if Self::evaluate_guards(&entry.tx.guards, epoch, db, contract_engine) {
                result.matured += 1;
                result.matured_txs.push((
                    entry.tx.submitter,
                    entry.tx.inner_tx_bytes.clone(),
                    entry.tx.gas_limit,
                ));
            } else {
                // Guards not satisfied yet — keep in queue.
                remaining.push(entry);
            }
        }

        result.pending = remaining.len();
        self.queue = remaining;
        result
    }

    /// Evaluate all temporal guards. Returns true if ALL guards pass.
    fn evaluate_guards<DB: StateDB + ?Sized>(
        guards: &[TemporalGuard],
        epoch: Epoch,
        db: &DB,
        contract_engine: &ContractEngine,
    ) -> bool {
        for guard in guards {
            match guard {
                TemporalGuard::AfterEpoch(e) => {
                    if epoch < *e {
                        return false;
                    }
                }
                TemporalGuard::BeforeEpoch(e) => {
                    if epoch >= *e {
                        return false;
                    }
                }
                TemporalGuard::EnergyBelow(obj_id, threshold) => {
                    if let Some(obj) = db.get_object(obj_id) {
                        let current = evaporchain_types::energy_at_epoch(
                            obj.energy,
                            obj.half_life,
                            epoch.saturating_sub(obj.last_refreshed),
                        );
                        if current >= *threshold {
                            return false;
                        }
                    } else {
                        // Object doesn't exist — treat as energy=0 (below any threshold).
                    }
                }
                TemporalGuard::EnergyAbove(obj_id, threshold) => {
                    if let Some(obj) = db.get_object(obj_id) {
                        let current = evaporchain_types::energy_at_epoch(
                            obj.energy,
                            obj.half_life,
                            epoch.saturating_sub(obj.last_refreshed),
                        );
                        if current <= *threshold {
                            return false;
                        }
                    } else {
                        return false; // Object gone = no energy = can't be above threshold.
                    }
                }
                TemporalGuard::ObjectEvaporated(obj_id) => {
                    if let Some(obj) = db.get_object(obj_id) {
                        if obj.state != evaporchain_types::ObjectState::Ghost {
                            return false;
                        }
                    }
                    // Object not found in active DB = likely evaporated.
                }
                TemporalGuard::ContractInPhase(contract_id, expected_phase) => {
                    if let Some(contract) = contract_engine.get(*contract_id) {
                        // Try to extract temporal state and check phase name.
                        if let Ok(ts) =
                            serde_json::from_value::<serde_json::Value>(contract.state.clone())
                        {
                            if let Some(phases) = ts.get("phases").and_then(|p| p.as_array()) {
                                let idx =
                                    ts.get("current_phase")
                                        .and_then(|v| v.as_u64())
                                        .unwrap_or(0) as usize;
                                if let Some(phase) = phases.get(idx) {
                                    let name =
                                        phase.get("name").and_then(|n| n.as_str()).unwrap_or("");
                                    if name != expected_phase {
                                        return false;
                                    }
                                } else {
                                    return false;
                                }
                            } else {
                                return false;
                            }
                        } else {
                            return false;
                        }
                    } else {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// Number of entries currently in the queue.
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// Whether the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

impl Default for DeferredQueue {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Decay Watcher Engine
// ═══════════════════════════════════════════════════════════════════════════

/// Result of processing watchers for one block.
#[derive(Debug, Default)]
pub struct WatcherResult {
    /// Number of watchers that fired.
    pub fired: usize,
    /// Callbacks to invoke: (contract_id, method, args).
    pub callbacks: Vec<(u64, String, String)>,
}

/// Monitors object energy levels and fires contract callbacks when thresholds are crossed.
///
/// Integrated into the per-block evaporation tick. After energy is decayed for all objects,
/// the watcher engine checks registered watchers and fires matching callbacks.
pub struct DecayWatcherEngine {
    watchers: Vec<EnergyWatcher>,
    next_id: u64,
}

impl DecayWatcherEngine {
    pub fn new() -> Self {
        Self {
            watchers: Vec::new(),
            next_id: 1,
        }
    }

    /// Register a new energy watcher. Returns the watcher ID.
    #[allow(clippy::too_many_arguments)]
    pub fn register(
        &mut self,
        object_id: ObjectId,
        threshold: Energy,
        fire_below: bool,
        callback_contract_id: u64,
        callback_method: String,
        callback_args: String,
        current_epoch: Epoch,
    ) -> Result<u64, TemporalError> {
        if self.watchers.len() >= MAX_WATCHERS {
            return Err(TemporalError::TooManyWatchers { max: MAX_WATCHERS });
        }

        let id = self.next_id;
        self.next_id += 1;

        self.watchers.push(EnergyWatcher {
            id,
            object_id,
            threshold,
            fire_below,
            callback_contract_id,
            callback_method,
            callback_args,
            fired: false,
            registered_epoch: current_epoch,
        });

        Ok(id)
    }

    /// Remove a watcher by ID.
    pub fn unregister(&mut self, watcher_id: u64) -> bool {
        let before = self.watchers.len();
        self.watchers.retain(|w| w.id != watcher_id);
        self.watchers.len() < before
    }

    /// Process all watchers against current object state.
    /// Called during the evaporation tick, after energy has been decayed.
    pub fn process<DB: StateDB + ?Sized>(&mut self, epoch: Epoch, db: &DB) -> WatcherResult {
        let mut result = WatcherResult::default();

        for watcher in &mut self.watchers {
            if watcher.fired {
                continue;
            }

            let current_energy = if let Some(obj) = db.get_object(&watcher.object_id) {
                evaporchain_types::energy_at_epoch(
                    obj.energy,
                    obj.half_life,
                    epoch.saturating_sub(obj.last_refreshed),
                )
            } else {
                0 // Object evaporated = energy 0.
            };

            let should_fire = if watcher.fire_below {
                current_energy < watcher.threshold
            } else {
                current_energy > watcher.threshold
            };

            if should_fire {
                watcher.fired = true;
                result.fired += 1;
                result.callbacks.push((
                    watcher.callback_contract_id,
                    watcher.callback_method.clone(),
                    watcher.callback_args.clone(),
                ));
            }
        }

        // Garbage-collect fired watchers.
        self.watchers.retain(|w| !w.fired);

        result
    }

    /// Number of active (unfired) watchers.
    pub fn active_count(&self) -> usize {
        self.watchers.len()
    }

    /// List all active watchers for a given object.
    pub fn watchers_for_object(&self, object_id: &ObjectId) -> Vec<&EnergyWatcher> {
        self.watchers
            .iter()
            .filter(|w| &w.object_id == object_id)
            .collect()
    }
}

impl Default for DecayWatcherEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_state::db::InMemoryStateDB;
    use evaporchain_types::{ObjectState, StateObject};

    fn make_deferred_tx(
        submitter: [u8; 32],
        guards: Vec<TemporalGuard>,
        inner_bytes: Vec<u8>,
    ) -> DeferredTx {
        DeferredTx {
            submitter,
            nonce: 0,
            deposit: 50_000,
            guards,
            inner_tx_bytes: inner_bytes,
            gas_limit: 100_000,
            signature: None,
            public_key: None,
        }
    }

    fn addr(seed: u8) -> [u8; 32] {
        let mut a = [0u8; 32];
        a[0] = seed;
        a
    }

    fn make_object(id: [u8; 32], energy: u64, half_life: u64, epoch: u64) -> StateObject {
        StateObject {
            id,
            owner: [0u8; 32],
            energy,
            half_life,
            created_at: 0,
            last_refreshed: epoch,
            state: ObjectState::Active,
            grace_epoch: None,
            data: vec![],
            decay_curve: None,
            lad_mode: None,
        }
    }

    // ─── DeferredQueue Tests ─────────────────────────────────────────

    #[test]
    fn test_deferred_submit_and_mature() {
        let mut queue = DeferredQueue::new();
        let contract_engine = ContractEngine::new();

        let tx = make_deferred_tx(
            addr(1),
            vec![TemporalGuard::AfterEpoch(5)],
            vec![0x01, 0x02],
        );
        queue.submit(tx).unwrap();
        assert_eq!(queue.len(), 1);

        // Epoch 3: too early.
        let db = InMemoryStateDB::new();
        let result = queue.process_epoch(3, &db, &contract_engine);
        assert_eq!(result.matured, 0);
        assert_eq!(result.pending, 1);

        // Epoch 5: should mature.
        let result = queue.process_epoch(5, &db, &contract_engine);
        assert_eq!(result.matured, 1);
        assert_eq!(result.matured_txs.len(), 1);
        assert_eq!(result.matured_txs[0].1, vec![0x01, 0x02]);
        assert_eq!(queue.len(), 0);
    }

    #[test]
    fn test_deferred_expiry() {
        let mut queue = DeferredQueue::new();
        let contract_engine = ContractEngine::new();

        let tx = make_deferred_tx(
            addr(1),
            vec![
                TemporalGuard::AfterEpoch(10),
                TemporalGuard::BeforeEpoch(20),
            ],
            vec![0xFF],
        );
        queue.submit(tx).unwrap();

        // Epoch 25: past expiry.
        let db = InMemoryStateDB::new();
        let result = queue.process_epoch(25, &db, &contract_engine);
        assert_eq!(result.expired, 1);
        assert_eq!(result.matured, 0);
        assert_eq!(result.refunds.len(), 1);
        assert_eq!(result.refunds[0].0, addr(1));
    }

    #[test]
    fn test_deferred_energy_below_guard() {
        let mut queue = DeferredQueue::new();
        let contract_engine = ContractEngine::new();
        let obj_id = addr(99);

        let tx = make_deferred_tx(
            addr(1),
            vec![TemporalGuard::EnergyBelow(obj_id, 500)],
            vec![0xAA],
        );
        queue.submit(tx).unwrap();

        // Object with 1000 energy, half_life=10, refreshed at epoch 0.
        let mut db = InMemoryStateDB::new();
        db.put_object(make_object(obj_id, 1000, 10, 0));

        // Epoch 1: energy ~950, still above 500.
        let result = queue.process_epoch(1, &db, &contract_engine);
        assert_eq!(result.matured, 0);

        // Epoch 15: energy ~293, below 500.
        let result = queue.process_epoch(15, &db, &contract_engine);
        assert_eq!(result.matured, 1);
    }

    #[test]
    fn test_deferred_object_evaporated_guard() {
        let mut queue = DeferredQueue::new();
        let contract_engine = ContractEngine::new();
        let obj_id = addr(50);

        let tx = make_deferred_tx(
            addr(1),
            vec![TemporalGuard::ObjectEvaporated(obj_id)],
            vec![0xBB],
        );
        queue.submit(tx).unwrap();

        // Object is Active — should not fire.
        let mut db = InMemoryStateDB::new();
        db.put_object(make_object(obj_id, 100, 10, 0));
        let result = queue.process_epoch(1, &db, &contract_engine);
        assert_eq!(result.matured, 0);

        // Mark object as Ghost.
        db.get_object_mut(&obj_id).unwrap().state = ObjectState::Ghost;
        let result = queue.process_epoch(2, &db, &contract_engine);
        assert_eq!(result.matured, 1);
    }

    #[test]
    fn test_deferred_reject_no_guards() {
        let mut queue = DeferredQueue::new();
        let tx = make_deferred_tx(addr(1), vec![], vec![0x01]);
        assert!(matches!(queue.submit(tx), Err(TemporalError::NoGuards)));
    }

    #[test]
    fn test_deferred_reject_low_deposit() {
        let mut queue = DeferredQueue::new();
        let mut tx = make_deferred_tx(addr(1), vec![TemporalGuard::AfterEpoch(5)], vec![0x01]);
        tx.deposit = 100; // Below MIN_DEFERRED_DEPOSIT.
        assert!(matches!(
            queue.submit(tx),
            Err(TemporalError::InsufficientDeposit { .. })
        ));
    }

    #[test]
    fn test_deferred_multiple_guards_all_must_pass() {
        let mut queue = DeferredQueue::new();
        let contract_engine = ContractEngine::new();
        let obj_id = addr(42);

        // Guard: AfterEpoch(5) AND EnergyBelow(obj, 200)
        let tx = make_deferred_tx(
            addr(1),
            vec![
                TemporalGuard::AfterEpoch(5),
                TemporalGuard::EnergyBelow(obj_id, 200),
            ],
            vec![0xCC],
        );
        queue.submit(tx).unwrap();

        let mut db = InMemoryStateDB::new();
        db.put_object(make_object(obj_id, 1000, 10, 0));

        // Epoch 5: epoch OK, but energy = 750 > 200.
        let result = queue.process_epoch(5, &db, &contract_engine);
        assert_eq!(result.matured, 0);

        // Epoch 30: energy = 125 (1000 >> 3), below 200. Both guards now pass.
        let result = queue.process_epoch(30, &db, &contract_engine);
        assert_eq!(result.matured, 1);
    }

    // ─── DecayWatcherEngine Tests ────────────────────────────────────

    #[test]
    fn test_watcher_fires_on_threshold() {
        let mut engine = DecayWatcherEngine::new();
        let obj_id = addr(10);

        engine
            .register(
                obj_id,
                500,
                true, // fire when below 500
                42,   // contract_id
                "on_low_energy".into(),
                "{}".into(),
                0,
            )
            .unwrap();
        assert_eq!(engine.active_count(), 1);

        let mut db = InMemoryStateDB::new();
        db.put_object(make_object(obj_id, 1000, 10, 0));

        // Epoch 1: energy ~950, above threshold.
        let result = engine.process(1, &db);
        assert_eq!(result.fired, 0);
        assert_eq!(engine.active_count(), 1);

        // Epoch 15: energy ~293, below 500 threshold.
        let result = engine.process(15, &db);
        assert_eq!(result.fired, 1);
        assert_eq!(result.callbacks.len(), 1);
        assert_eq!(result.callbacks[0].0, 42);
        assert_eq!(result.callbacks[0].1, "on_low_energy");

        // Watcher is one-shot: garbage-collected after firing.
        assert_eq!(engine.active_count(), 0);
    }

    #[test]
    fn test_watcher_fires_above_threshold() {
        let mut engine = DecayWatcherEngine::new();
        let obj_id = addr(20);

        engine
            .register(
                obj_id,
                100,
                false, // fire when ABOVE 100
                99,
                "on_high_energy".into(),
                r#"{"alert": true}"#.into(),
                0,
            )
            .unwrap();

        let mut db = InMemoryStateDB::new();
        db.put_object(make_object(obj_id, 500, 10, 0));

        // Epoch 0: energy 500, above 100 → fires immediately.
        let result = engine.process(0, &db);
        assert_eq!(result.fired, 1);
    }

    #[test]
    fn test_watcher_unregister() {
        let mut engine = DecayWatcherEngine::new();
        let id = engine
            .register(addr(1), 100, true, 1, "m".into(), "{}".into(), 0)
            .unwrap();
        assert_eq!(engine.active_count(), 1);
        assert!(engine.unregister(id));
        assert_eq!(engine.active_count(), 0);
        assert!(!engine.unregister(id)); // Already removed.
    }

    #[test]
    fn test_watcher_evaporated_object() {
        let mut engine = DecayWatcherEngine::new();
        let obj_id = addr(30);

        engine
            .register(obj_id, 1, true, 50, "on_evap".into(), "{}".into(), 0)
            .unwrap();

        // Object not in DB = evaporated = energy 0 < 1.
        let db = InMemoryStateDB::new();
        let result = engine.process(100, &db);
        assert_eq!(result.fired, 1);
    }

    #[test]
    fn test_multiple_watchers_on_same_object() {
        let mut engine = DecayWatcherEngine::new();
        let obj_id = addr(40);

        // Watcher 1: fire below 500
        engine
            .register(obj_id, 500, true, 1, "low".into(), "{}".into(), 0)
            .unwrap();
        // Watcher 2: fire below 100
        engine
            .register(obj_id, 100, true, 2, "critical".into(), "{}".into(), 0)
            .unwrap();

        let mut db = InMemoryStateDB::new();
        db.put_object(make_object(obj_id, 1000, 10, 0));

        // Epoch 15: energy = 375 (500>>1 - frac). Below 500 but above 100.
        let result = engine.process(15, &db);
        assert_eq!(result.fired, 1);
        assert_eq!(result.callbacks[0].1, "low");
        assert_eq!(engine.active_count(), 1); // "critical" still active

        // Epoch 40: energy = 62 (1000>>4). Below 100.
        let result = engine.process(40, &db);
        assert_eq!(result.fired, 1);
        assert_eq!(result.callbacks[0].1, "critical");
        assert_eq!(engine.active_count(), 0);
    }
}
