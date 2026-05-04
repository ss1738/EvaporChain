//! Thermal Software Transactional Memory.
//!
//! Optimistic concurrency over chain state. Each transaction
//! declares a read-set + write-set up-front; the scheduler runs
//! them in parallel; commit validates that no other committed tx
//! has touched any read-set entry; conflicts resolve by energy
//! priority (higher-energy tx wins). Lower-energy losers retry
//! against the post-commit state.
//!
//! ## Why "thermal"
//!
//! In chemistry / statistical physics, *thermal* describes a
//! system whose dynamics are dominated by energy-weighted
//! transitions: the higher-energy state preempts the lower in
//! reactions / kinetic-Monte-Carlo simulations.
//!
//! At L1, the same gives:
//! - Higher-energy transactions get priority in conflicts.
//! - Lower-energy transactions decay into the abort/retry path.
//! - Deadlocks are impossible: any cycle in the conflict graph
//!   is broken by the strict total order on energy (with a
//!   deterministic tie-breaker on tx-id hash).
//!
//! ## Three structural decisions enforced as invariants
//!
//! 1. **Validator-deterministic schedule.** Two validators
//!    receiving the same transaction batch in any order produce
//!    the byte-identical commit sequence and final state. Tx
//!    sort key is `(−energy, tx_id_hash)` lexicographic — pure
//!    integer comparison.
//!
//! 2. **No deadlocks.** The conflict-resolution rule is a strict
//!    total order: higher-energy wins; equal-energy → lower
//!    `tx_id_hash` wins. Any conflict graph has a unique
//!    topological order.
//!
//! 3. **Atomic commit.** A transaction either commits all of its
//!    write-set or none of it. The scheduler buffers writes
//!    until the conflict check passes; aborted transactions
//!    have zero effect on state.
//!
//! ## What this crate does NOT do
//!
//! - It does NOT execute the transaction's compute. Caller passes
//!   a closure / opaque payload; this layer schedules.
//! - It does NOT do gas / fee accounting. That's the chain's
//!   transaction layer.
//! - It does NOT model contention-aware retry policies. V1
//!   aborted tx returns to the caller for retry against a fresh
//!   snapshot; the chain's mempool decides whether to resubmit.
//!
//! ## Module map
//!
//! - [`tx`] — [`Tx`] + [`StateKey`] + [`StateValue`].
//! - [`scheduler`] — [`ThermalScheduler`] + commit / abort logic.
//! - [`order`] — the energy-priority comparator + total-order
//!   witness.

pub mod order;
pub mod scheduler;
pub mod tx;

pub use order::compare_thermal;
pub use scheduler::{CommitOutcome, ScheduleError, ThermalScheduler};
pub use tx::{StateKey, StateValue, Tx, TxId, TxOutcome};
