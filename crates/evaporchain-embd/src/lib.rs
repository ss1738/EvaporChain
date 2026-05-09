//! Earth-Mover Block Diff (EMBD) — MEV-detection primitive.
//!
//! ## What this is
//!
//! Two views of the same block:
//! - **Arrival view**: the order in which the chain saw transactions
//!   enter the mempool (each tx tagged with `arrival_rank`).
//! - **Final view**: the order in which the block producer placed
//!   transactions in the block (`final_rank` 0..N-1).
//!
//! EMBD computes the **L1-Wasserstein distance** between the
//! arrival-rank distribution and the final-rank distribution under
//! the rank metric `d(i, j) = |i − j|`. For 1D rank-permutation
//! comparisons, this has a closed form:
//!
//! ```text
//! EMBD = Σ_t |arrival_rank(t) − final_rank(t)|
//! ```
//!
//! divided by the count of transactions to give a per-tx average
//! displacement. The chain compares this against a configurable
//! `embd_floor`; blocks above the floor are flagged as suspect.
//!
//! ## Why this is sound for MEV detection
//!
//! Honest ordering: producers order by fee + tie-break by arrival.
//! The arrival vs final order should differ only modestly (some
//! re-ordering for higher-fee txs, but bounded). Adversarial
//! ordering (sandwich attacks, frontrunning, backrunning): the
//! attacker INSERTS new transactions out of arrival order, or
//! REORDERS arrival-already-present txs to bracket a victim. Both
//! produce large displacement on individual txs.
//!
//! ## Three structural decisions enforced as tests
//!
//! 1. **Pure-integer fixed-point.** All arithmetic in u128;
//!    EMBD result in u64 micros (per-tx average displacement,
//!    scaled by 10⁶).
//!
//! 2. **Symmetric.** EMBD(arrival, final) == EMBD(final, arrival)
//!    — earth-mover distance is a metric.
//!
//! 3. **Bounded by block-size minus one.** The maximum possible
//!    EMBD on N transactions is `floor((N²-1)/2N) · MICROS` (when
//!    one tx jumps from rank 0 to rank N-1). Asserted as
//!    `embd_max_for_block_size(n)` for the chain to validate
//!    against.
//!
//! ## What this crate does NOT do
//!
//! - Does NOT compute the arrival ordering. Caller passes the
//!   arrival-rank vector + the final-rank vector.
//! - Does NOT model fee-based reordering as "honest" — pure
//!   displacement metric. The chain's higher layer applies a
//!   fee-aware floor.
//! - Does NOT identify *which* transactions caused the spike.
//!   That's a debug helper, V2.
//!
//! ## Module map
//!
//! - [`block_diff`] — [`embd_micros`] driver +
//!   [`embd_floor_check`] gate + [`embd_max_for_block_size`].

pub mod block_diff;

pub use block_diff::{
    embd_floor_check, embd_max_for_block_size, embd_micros, BlockDiffError, MICROS,
};
