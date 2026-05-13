//! Composition theorems — the privacy-budget arithmetic.
//!
//! The chain accepts **basic composition** as a witness type (V1).
//! Advanced composition (Dwork-Rothblum-Vadhan 2010) and Renyi-DP
//! composition are V2 — they require f64 transcendentals (`sqrt`,
//! `ln`, `exp`) which don't have validator-deterministic behaviour
//! across architectures without careful work.

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CompositionError {
    #[error("composition arithmetic overflowed u64")]
    Overflow,
    #[error("composition over zero queries — degenerate input")]
    ZeroQueries,
}

/// Basic composition (Dwork-McSherry-Nissim-Smith 2006):
///
/// **Theorem.** A k-fold adaptive composition of `(ε_i, δ_i)`-DP
/// mechanisms is `(Σ ε_i, Σ δ_i)`-DP.
///
/// All-integer arithmetic. Bounded by saturating addition; we
/// surface overflow as an error rather than silently saturating.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BasicComposition {
    pub total_epsilon_micros: u64,
    pub total_delta_ppb: u64,
    pub k: u64,
}

impl BasicComposition {
    /// Compose the given list of `(ε_i, δ_i)` queries.
    pub fn compose(queries: &[(u64, u64)]) -> Result<Self, CompositionError> {
        if queries.is_empty() {
            return Err(CompositionError::ZeroQueries);
        }
        let mut total_eps: u64 = 0;
        let mut total_delta: u64 = 0;
        for (e, d) in queries {
            total_eps = total_eps
                .checked_add(*e)
                .ok_or(CompositionError::Overflow)?;
            total_delta = total_delta
                .checked_add(*d)
                .ok_or(CompositionError::Overflow)?;
        }
        Ok(Self {
            total_epsilon_micros: total_eps,
            total_delta_ppb: total_delta,
            k: queries.len() as u64,
        })
    }
}

/// Compute `(ε_total, δ_total)` for k queries each at `(ε, δ)` via
/// basic composition. Pure integer.
pub fn basic_composition_epsilon(
    epsilon_micros: u64,
    delta_ppb: u64,
    k: u64,
) -> Result<(u64, u64), CompositionError> {
    if k == 0 {
        return Err(CompositionError::ZeroQueries);
    }
    let total_eps = epsilon_micros
        .checked_mul(k)
        .ok_or(CompositionError::Overflow)?;
    let total_delta = delta_ppb.checked_mul(k).ok_or(CompositionError::Overflow)?;
    Ok((total_eps, total_delta))
}

/// Advanced composition (Dwork-Rothblum-Vadhan 2010) — V2 stub.
///
/// **Theorem.** For any `δ' > 0`, the k-fold composition of
/// `(ε, δ)`-DP mechanisms is also `(ε', kδ + δ')`-DP with
/// `ε' = ε √(2k ln(1/δ')) + kε(eᵉ - 1)`.
///
/// The bound is tighter than basic for large k. V1 ships only the
/// witness type; the actual computation is V2 (requires f64
/// transcendentals + a validator-determinism story).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdvancedComposition {
    pub total_epsilon_micros: u64,
    pub total_delta_ppb: u64,
    pub k: u64,
    pub delta_prime_ppb: u64,
}

/// V1: not implemented. The function exists so V2 implementers can
/// fill in the body without changing the public surface.
pub fn advanced_composition_epsilon(
    _epsilon_micros: u64,
    _delta_ppb: u64,
    _k: u64,
    _delta_prime_ppb: u64,
) -> Result<(u64, u64), CompositionError> {
    // Intentionally not implemented in V1. Call basic_composition
    // and accept the looser bound. V2 will return the tighter
    // (ε', δ') with a validator-deterministic computation.
    Err(CompositionError::Overflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_composition_sum_of_queries() {
        let queries = vec![(100_000, 10), (200_000, 20), (150_000, 15)];
        let c = BasicComposition::compose(&queries).unwrap();
        assert_eq!(c.total_epsilon_micros, 450_000);
        assert_eq!(c.total_delta_ppb, 45);
        assert_eq!(c.k, 3);
    }

    #[test]
    fn basic_composition_fixed_epsilon() {
        // 10 queries × 0.1 each → ε = 1.0 total.
        let (eps, delta) = basic_composition_epsilon(100_000, 0, 10).unwrap();
        assert_eq!(eps, 1_000_000);
        assert_eq!(delta, 0);
    }

    #[test]
    fn basic_composition_zero_queries_rejected() {
        let err = BasicComposition::compose(&[]).unwrap_err();
        assert_eq!(err, CompositionError::ZeroQueries);

        let err = basic_composition_epsilon(100_000, 0, 0).unwrap_err();
        assert_eq!(err, CompositionError::ZeroQueries);
    }

    #[test]
    fn basic_composition_overflow_surfaces() {
        let err = basic_composition_epsilon(u64::MAX / 2, 0, 4).unwrap_err();
        assert_eq!(err, CompositionError::Overflow);
    }

    #[test]
    fn basic_composition_iterative_matches_one_shot() {
        // Adding queries one at a time matches summing them at once.
        let queries: Vec<(u64, u64)> = (1..=5).map(|i| (i * 10_000, i * 5)).collect();
        let one_shot = BasicComposition::compose(&queries).unwrap();
        let mut acc_eps: u64 = 0;
        let mut acc_d: u64 = 0;
        for (e, d) in &queries {
            acc_eps += e;
            acc_d += d;
        }
        assert_eq!(one_shot.total_epsilon_micros, acc_eps);
        assert_eq!(one_shot.total_delta_ppb, acc_d);
    }

    #[test]
    fn advanced_composition_v2_stubbed() {
        // V1 returns Err — V2 implementers fill this in.
        assert!(advanced_composition_epsilon(100_000, 100, 100, 1).is_err());
    }

    /// T1.20 — `BasicComposition::compose` surfaces overflow on the
    /// DELTA accumulator too, not just epsilon. Existing
    /// `basic_composition_overflow_surfaces` only tests the multiply
    /// path on epsilon. This pins the additive-delta-overflow path
    /// (lines 46-48) explicitly.
    #[test]
    fn t1_20_compose_delta_overflow_surfaces() {
        // Two huge deltas that sum past u64::MAX.
        let queries = vec![(0, u64::MAX - 5), (0, 10)];
        let err = BasicComposition::compose(&queries).unwrap_err();
        assert_eq!(err, CompositionError::Overflow);
    }

    /// T1.20 — `basic_composition_epsilon` surfaces overflow on the
    /// delta multiply too (line 71). Existing test only covered the
    /// epsilon multiply overflow.
    #[test]
    fn t1_20_basic_composition_epsilon_delta_overflow_surfaces() {
        // delta = u64::MAX/2, k = 4 → delta × k overflows.
        let err = basic_composition_epsilon(0, u64::MAX / 2, 4).unwrap_err();
        assert_eq!(err, CompositionError::Overflow);
    }

    /// T1.20 — the two compose APIs agree:
    /// `compose([(ε,δ); k])` matches `basic_composition_epsilon(ε, δ, k)`.
    /// Pins that callers can use either form interchangeably; a future
    /// refactor of one path that diverged from the other would trip
    /// this.
    #[test]
    fn t1_20_compose_and_basic_composition_epsilon_agree() {
        let eps = 75_000u64;
        let delta = 25u64;
        let k = 8u64;
        // Vec form.
        let queries: Vec<(u64, u64)> = (0..k).map(|_| (eps, delta)).collect();
        let c = BasicComposition::compose(&queries).unwrap();
        // Multiplicative form.
        let (m_eps, m_delta) = basic_composition_epsilon(eps, delta, k).unwrap();
        assert_eq!(c.total_epsilon_micros, m_eps);
        assert_eq!(c.total_delta_ppb, m_delta);
        assert_eq!(c.k, k);
    }

    /// T1.20 — `compose` accepts a single-query input (k=1) and
    /// returns the unchanged budget. Pins the boundary where the
    /// composition theorem degenerates to identity.
    #[test]
    fn t1_20_compose_single_query_returns_identity() {
        let c = BasicComposition::compose(&[(42_000, 7)]).unwrap();
        assert_eq!(c.total_epsilon_micros, 42_000);
        assert_eq!(c.total_delta_ppb, 7);
        assert_eq!(c.k, 1);
    }

    proptest::proptest! {
        #[test]
        fn property_basic_composition_is_monotone(eps in 0u64..1_000_000u64, k in 1u64..1000u64) {
            // Adding one more query never decreases ε_total.
            let (e1, _) = basic_composition_epsilon(eps, 0, k).unwrap();
            let (e2, _) = basic_composition_epsilon(eps, 0, k + 1).unwrap();
            proptest::prop_assert!(e2 >= e1);
        }

        #[test]
        fn property_basic_composition_associative(eps_a in 1u64..100_000u64, eps_b in 1u64..100_000u64) {
            // ε(query a) + ε(query b) = ε(query b) + ε(query a)
            let (s1, _) = BasicComposition::compose(&[(eps_a, 0), (eps_b, 0)])
                .map(|c| (c.total_epsilon_micros, c.total_delta_ppb))
                .unwrap();
            let (s2, _) = BasicComposition::compose(&[(eps_b, 0), (eps_a, 0)])
                .map(|c| (c.total_epsilon_micros, c.total_delta_ppb))
                .unwrap();
            proptest::prop_assert_eq!(s1, s2);
        }
    }
}
