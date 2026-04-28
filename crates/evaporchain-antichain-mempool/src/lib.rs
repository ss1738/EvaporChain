//! Evap-Antichain Mempool — the mempool *is* the partial order.
//!
//! Per `research/INVENTION_STACK.md` §4.1 row 2:
//!
//! > **Evap-Antichain Mempool** — Mempool *is* the partial order;
//! > producer extends maximal antichains whose total energy clears a
//! > threshold.
//!
//! ## What an antichain is
//!
//! Given a partial order on blocks (the [`evaporchain_light_cone::LightCone`]
//! DAG), an *antichain* is a set of blocks no two of which are
//! comparable — i.e. every pair is *concurrent*. A *maximal* antichain
//! cannot be extended by adding another block without breaking
//! mutual-concurrency.
//!
//! ## Why this is the right mempool shape
//!
//! Two pending payloads that have no causal relation can be packed
//! together into the same proposal without re-ordering risk: their
//! aggregate energy is the sum of the individual energies and the
//! producer can include them in either temporal order. The mempool's
//! job is to surface a *maximal* antichain whose total energy clears
//! the chain-set threshold (the "block enough work has accrued" test).
//!
//! ## Module map
//!
//! - [`antichain`] — `Antichain` newtype + invariant check.
//! - [`maximal`] — `is_maximal_antichain` and `extend_to_maximal`
//!   (greedy by descending energy).
//! - [`threshold`] — `total_energy_meets_threshold` for the proposer
//!   inclusion gate.

pub mod antichain;
pub mod maximal;
pub mod threshold;

pub use antichain::{Antichain, AntichainError};
pub use maximal::{extend_to_maximal, is_maximal_antichain};
pub use threshold::total_energy_meets_threshold;
