//! Bell-Certified Beacon (Tier 2).
//!
//! Per `research/INVENTION_STACK.md` §4.2:
//!
//! > **Bell-Certified Beacon** — Device-independent randomness from
//! > CHSH Bell tests; consumed by Decay-Lamport Time.
//!
//! ## CHSH inequality
//!
//! For any local-realist correlation, the CHSH "S value" is bounded:
//!
//! ```text
//!   |S| = |E(a, b) - E(a, b') + E(a', b) + E(a', b')|  ≤  2
//! ```
//!
//! Quantum-entangled systems can violate this bound (Tsirelson's
//! bound: `|S| ≤ 2√2 ≈ 2.828`). A measured `|S| > 2` is *device-
//! independent* certification that the source is genuinely random
//! (no local hidden variable can fake it).
//!
//! ## What this substrate ships
//!
//! - [`chsh`] — `chsh_s_value(e_ab, e_ab_prime, e_a_prime_b,
//!   e_a_prime_b_prime)` returning the S value in milli-units (×1000)
//!   for integer arithmetic.
//! - [`gate`] — `bell_certified(s_value_milli, threshold_milli)`
//!   returns true iff |S| × 1000 > threshold (default threshold
//!   2000 = the local-realism boundary).
//!
//! Production wires a real entangled-photon source (or reuses an
//! external CHSH-certified randomness service like NIST's beacon)
//! via the chosen `s_value_milli` plumbing. The on-chain rule is the
//! same: reject any beacon emission whose CHSH-S value is at or below
//! the local-realism boundary.

pub mod chsh;
pub mod gate;

pub use chsh::{chsh_s_value, ChshError};
pub use gate::{bell_certified, LOCAL_REALISM_S_MILLI};
