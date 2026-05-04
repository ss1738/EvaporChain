//! Causal-CHSH — Bell-style cartel-detection bound for blockchain
//! causal sets.
//!
//! ## What this is
//!
//! Bell's CHSH inequality (Clauser-Horne-Shimony-Holt 1969) detects
//! hidden coordination between physically-separated quantum systems by
//! measuring whether four correlation terms can violate the classical
//! bound `|S| ≤ 2`. EvaporChain's LightCone DAG produces the same
//! structural setup: pairs of *concurrent blocks* (mutually
//! incomparable in the partial order — neither precedes the other).
//! Honest concurrent producers cannot coordinate (no causal channel
//! between them, by definition of concurrent). A cartel that *does*
//! coordinate violates the inequality.
//!
//! ## The math
//!
//! Pick two binary observables `(A, A')` measured on the *first*
//! concurrent block of each pair, and two `(B, B')` on the *second*.
//! Each observable maps a block to ±1 (e.g., sign of energy minus
//! median, sign of tx-count minus median). Form four correlation
//! terms by averaging over a sample of concurrent pairs `{(a_i, b_i)}`:
//!
//! ```text
//!   E(A , B ) = mean_i  A (a_i)·B (b_i)
//!   E(A , B') = mean_i  A (a_i)·B'(b_i)
//!   E(A', B ) = mean_i  A'(a_i)·B (b_i)
//!   E(A', B') = mean_i  A'(a_i)·B'(b_i)
//! ```
//!
//! The CHSH-style **causal-CHSH statistic**:
//!
//! ```text
//!   S = | E(A,B) + E(A,B') + E(A',B) − E(A',B') |
//! ```
//!
//! **Theorem (proposed):** Under honest-validator + LightCone causality
//! + EvaporChain's single-λ decay, `S ≤ 2`.
//! **Violation `S > 2` ⇒ hidden cross-validator coordination.**
//!
//! Where Bell's theorem gave physics quantum-entanglement detection,
//! Causal-CHSH gives blockchain *cartel-detection* with a closed-form
//! bound — not a heuristic, not a slashing rule, a *theorem*.
//!
//! ## Why no other chain can do this
//!
//! Causal-CHSH requires **concurrent blocks as a primitive**. Tendermint
//! (linear chain) has no concurrent blocks. Ethereum's reorgs are
//! competing finalisers, not concurrent producers. Avalanche's DAG is
//! unstructured (no partial order on blocks per validator-set
//! signoff). Only LightCone-style chains can even *form* the four-term
//! correlation. The math is new because the substrate is new.
//!
//! ## Doctrine status
//!
//! ⚠ **GATED** — `INVENTION_STACK.md` will reserve a Tier-0-supporting
//! row pending the empirical gate. The gate must show:
//!
//! - `S_honest < 1.8` on real Ethereum (or any honest L1) — the
//!   inequality has empirical headroom under honest traffic
//! - `S_cartel > 2.2` on synthetic coordinated traffic — the
//!   inequality actually separates coordination from non-coordination
//! - `gap = S_cartel − S_honest > 0.4` — the discrimination has signal
//!
//! If all three: ship as a Tier-0 supporting primitive (cartel
//! detector). If any fail: drop. Locked into INVENTION_STACK.md before
//! running, same MERA-style discipline.
//!
//! This commit lands the math primitive + synthetic gate. Real-data
//! gate is the next lane (Lane O.2).

pub mod alarm;
pub mod chsh;
pub mod gate;
pub mod trace;

pub use alarm::{AlarmStatus, CartelAlarm};
pub use chsh::{compute_chsh_s, compute_chsh_s_milli, ChshError, ConcurrentPair};
pub use gate::{run_synthetic_gate, GateThresholds, GateVerdict};
pub use trace::{extract_chsh_samples, synthesize_max_cartel_samples, BlockSummary};
