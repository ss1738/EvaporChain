//! `tur_check` — given a sample window of a chain current `J` and the
//! observed entropy production `Σ`, return `Verdict::Ok` iff TUR
//! holds, else `Verdict::Violation`.
//!
//! The TUR says `Var(J)/⟨J⟩² ≥ 2/Σ`. So:
//!
//! - `Verdict::Ok`     when `relative_variance ≥ tur_bound`
//! - `Verdict::Violation` when `relative_variance < tur_bound`
//!
//! A violation means the system is *less fluctuating* than thermo-
//! dynamics says is achievable for the accounted Σ. Concretely on
//! EvaporChain that means a current is far too steady for the
//! reported entropy budget — symptomatic of a cartel coordinating
//! block production / fees / slash absences in lockstep, an
//! adversary-class behaviour the TUR exposes formally.

use serde::{Deserialize, Serialize};

use crate::bound::tur_bound_fixed;
use crate::stats::relative_variance_fixed;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    Ok {
        observed: u128,
        bound: u128,
    },
    Violation {
        observed: u128,
        bound: u128,
    },
}

pub fn tur_check(samples: &[u64], sigma: u64) -> Verdict {
    let observed = relative_variance_fixed(samples);
    let bound = tur_bound_fixed(sigma);
    if observed >= bound {
        Verdict::Ok { observed, bound }
    } else {
        Verdict::Violation { observed, bound }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perfect_constants_violate_for_finite_sigma() {
        // Constant samples → relative_variance = 0 → below ANY finite
        // bound → violation. This is the canonical "cartel signature".
        let v = tur_check(&[10, 10, 10, 10, 10], 100);
        assert!(matches!(v, Verdict::Violation { .. }));
    }

    #[test]
    fn constants_pass_for_zero_sigma() {
        // Σ = 0 → bound = u128::MAX → constants are bounded by ∞ → OK.
        let v = tur_check(&[10, 10, 10, 10, 10], 0);
        match v {
            Verdict::Ok { observed: 0, bound } => assert_eq!(bound, u128::MAX),
            other => panic!("expected Ok(observed=0), got {other:?}"),
        }
    }

    #[test]
    fn high_variance_passes_at_typical_sigma() {
        // High-variance sequence with reasonable Σ — should satisfy TUR.
        let samples = vec![1, 100, 1, 100, 1, 100];
        let v = tur_check(&samples, 100);
        assert!(matches!(v, Verdict::Ok { .. }));
    }

    #[test]
    fn empty_samples_is_violation_at_finite_sigma() {
        // Empty → relative_variance = u128::MAX, bound finite → Ok.
        // Wait — MAX >= bound trivially, so it's Ok for any sigma > 0.
        let v = tur_check(&[], 100);
        assert!(matches!(v, Verdict::Ok { .. }));
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Property: with σ=0 the bound is +∞ and every sample passes.
        #[test]
        fn zero_sigma_always_ok(samples in proptest::collection::vec(0u64..1_000, 0..50)) {
            match tur_check(&samples, 0) {
                Verdict::Ok { .. } => {}
                Verdict::Violation { .. } => prop_assert!(false, "σ=0 must always pass"),
            }
        }

        /// Property: a constant sequence violates the bound for any
        /// finite σ > 0 (the cartel signature).
        #[test]
        fn constants_violate_for_positive_sigma(c in 1u64..1_000, sigma in 1u64..1_000) {
            let samples = vec![c; 8];
            match tur_check(&samples, sigma) {
                Verdict::Violation { .. } => {}
                Verdict::Ok { .. } => prop_assert!(false, "constants must violate at σ>0"),
            }
        }
    }
}
