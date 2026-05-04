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

#[cfg(test)]
mod press_claim_tests {
    use super::*;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "Cμ-Gate enforces Shalizi-Crutchfield 2001:
    /// `Cμ ≤ E + hμ`. Observed `Cμ` strictly above the bound is a
    /// structural fault (Sybil-like complexity-without-prediction).
    /// `Cμ` exactly at the bound is admitted; `Cμ = 0, E = 0, hμ = 0`
    /// is admitted; the entropy estimator rejects empty-count input."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        // At-bound admitted.
        assert!(matches!(
            cmu_check(300, 100, 200),
            Verdict::Ok { observed_cmu: 300, bound: 300 }
        ));
        // Below-bound admitted.
        assert!(matches!(cmu_check(250, 100, 200), Verdict::Ok { .. }));
        // Above-bound violation.
        assert!(matches!(
            cmu_check(301, 100, 200),
            Verdict::Violation { .. }
        ));
        // Zero across the board admitted.
        assert!(matches!(cmu_check(0, 0, 0), Verdict::Ok { .. }));
        // Cμ > 0 with zero bound → violation.
        assert!(matches!(cmu_check(1, 0, 0), Verdict::Violation { .. }));

        // bound = E + hμ.
        assert_eq!(cmu_bound(100, 200), 300);
        // Saturating: even at u64::MAX inputs, bound doesn't panic.
        assert_eq!(cmu_bound(u64::MAX, 1), u64::MAX);

        // Entropy estimator: deterministic distribution → 0 millibits.
        assert_eq!(entropy_millibits(&[100, 0, 0]).unwrap(), 0);
        // Empty input fails closed.
        assert!(entropy_millibits(&[]).is_err());
        // Uniform 50/50 → 1000 millibits (1 bit).
        assert_eq!(entropy_millibits(&[100, 100]).unwrap(), 1_000);
    }
}
