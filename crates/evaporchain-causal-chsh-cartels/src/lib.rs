//! Lane O.3 — realistic cartel-injection models for Causal-CHSH.
//!
//! The synthetic gate (V1) and the real-data runner (Lane O.2) both
//! ship a *static max-violation* cartel: rig three setting-pairs to +1
//! products and the fourth to −1, S = 4 exactly. Useful for proving
//! the gate machinery works, not realistic.
//!
//! This lane ships three deterministic cartel models that approximate
//! adversaries an honest chain might actually face:
//!
//! 1. **Coordinated-subset** — only K of N pairs are rigged; remaining
//!    N−K pairs use the honest sample. Tunable detection threshold:
//!    K must be large enough for the cartel S to clear the gate floor.
//!
//! 2. **Biased-coin** — every sample independently flipped to the
//!    violation pattern with probability p ∈ [0.5, 1.0], else honest.
//!    Smooth interpolation: p=0.5 ≈ honest, p=1.0 ≈ static cartel.
//!
//! 3. **PR-box approximation** — Popescu-Rohrlich box at visibility
//!    v ∈ [0, 1]. Sample distribution mirrors PR-box correlations
//!    (S = 4·v). v=1.0 → S=4 (algebraic max), v=2/√2 ≈ 0.707 → S=2√2
//!    (Tsirelson bound), v=0.5 → S=2 (classical bound).
//!
//! All models are validator-deterministic: a `[u8; 32]` seed plus the
//! honest sample fully determines the cartel sample. RNG is BLAKE3 in
//! XOF mode keyed by seed; no `rand` crate dependency.

pub mod models;
pub mod rng;

pub use models::{biased_coin_cartel, coordinated_subset_cartel, pr_box_cartel, CartelError};
