//! Cμ-Gate.
//!
//! Per `research/INVENTION_STACK.md` §A1.3 (Tier-0 supporting):
//!
//! > **Cμ-Gate** | Shalizi-Crutchfield identity `Cμ ≤ E + hμ` |
//! > block header carries Cμ; consensus rejects ΔCμ violations
//! > (Sybil/spam detector, principled τ from theorem).
//!
//! ## What the three quantities mean
//!
//! - **`Cμ` (statistical complexity)** — `H[ε(x)]`, the entropy of
//!   the *causal-state* distribution. The minimum information a
//!   predictor needs to carry from past to make optimal future
//!   predictions.
//! - **`E` (excess entropy)** — `I(past; future)`, the mutual
//!   information between past and future of the process. Bound on the
//!   *predictively useful* information in the past.
//! - **`hμ` (entropy rate)** — `H(X_n | X_{<n})`, the per-step
//!   irreducible uncertainty given the entire past.
//!
//! Shalizi-Crutchfield 2001 proves the identity `Cμ ≤ E + hμ`. Any
//! observed `Cμ > E + hμ` is a structural fault — the process is
//! generating apparent complexity that exceeds what's predictively
//! useful + what's intrinsically random. On EvaporChain that pattern
//! shows up as Sybil-like block production: many accounts producing
//! statistically-distinct headers without contributing predictively.
//!
//! ## What this crate ships
//!
//! - [`bound`] — `cmu_bound(e, h_mu) = E + hμ` and `cmu_check(observed,
//!   e, h_mu)` returning [`Verdict::Ok`] or [`Verdict::Violation`].
//! - [`estimator`] — `entropy_millibits(samples)` integer estimator
//!   over a finite-alphabet sample window, in millibits. Caller can
//!   feed this into all three of `Cμ`, `E`, `hμ` from chain-side
//!   sample streams (validator-id histogram, block-content histogram,
//!   etc.). Production sources for E and hμ live outside this crate.

pub mod bound;
pub mod estimator;

pub use bound::{cmu_bound, cmu_check, Verdict};
pub use estimator::{entropy_millibits, EntropyError};
