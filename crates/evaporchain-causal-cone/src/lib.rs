//! Causal-Cone Validator State.
//!
//! Per `research/INVENTION_STACK.md` §A1.3 (Tier-0 supporting):
//!
//! > **Causal-Cone Validator State** | Shalizi 2003 (light-cone
//! > sufficient statistics) | upgrades Light-Cone Consensus from
//! > heuristic to theorem-backed via the same Optimal Prediction
//! > Theorem.
//!
//! ## What this gives the chain
//!
//! Shalizi-Crutchfield's *Optimal Prediction Theorem* says: the
//! minimal sufficient statistic for predicting a system's future from
//! its past is the equivalence class of "causally identical" pasts —
//! the *causal state*. For chain-state-prediction this means a
//! validator only needs a *constant-size* summary of its past
//! light-cone (ancestors, energy, observation-time bounds, canonical
//! cone hash) to make every prediction the full history would allow.
//!
//! That's a huge bandwidth win for light clients and for every
//! consensus message that today carries a header chain.
//!
//! ## What is the summary
//!
//! Six fields, all `u64`-or-narrower so the on-wire size is bounded:
//!
//! ```rust,ignore
//! pub struct CausalConeSummary {
//!     pub head_id:                    BlockId,        // 32 bytes
//!     pub ancestor_count:             u64,            // size of past
//!     pub total_remaining_energy:     u128,           // λ-decayed sum
//!     pub oldest_observed_epoch:      u64,
//!     pub latest_observed_epoch:      u64,
//!     pub canonical_cone_hash:        [u8; 32],       // blake3
//! }
//! ```
//!
//! Two cones are *causally equivalent* (in Shalizi's sense) iff their
//! summaries are equal. The `canonical_cone_hash` field is the strict
//! identity gate: equal hashes ⇒ equal cones.
//!
//! ## Module map
//!
//! - [`summary`] — `CausalConeSummary` + `summarize_cone`.
//! - [`canonical`] — `canonical_cone_hash` (blake3 over a sorted,
//!   domain-separated serialization of the cone's block ids).

pub mod canonical;
pub mod summary;

pub use canonical::canonical_cone_hash;
pub use summary::{summarize_cone, CausalConeSummary, SummaryError};
