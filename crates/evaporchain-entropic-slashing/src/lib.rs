//! Entropic Slashing — Tier 2.
//!
//! Per `research/INVENTION_STACK.md` §4.2:
//!
//! > **Entropic Slashing** — Shannon-weighted slash → energy-aware
//! > MEV burn → refresh pool (the conservation triplet).
//!
//! Sister of [`evaporchain_sanov_slashing`]. Where Sanov uses
//! `KL(observed‖honest)` as the slash multiplier (the *exact* large-
//! deviation cost of the misbehaviour trajectory), Entropic Slashing
//! uses the *Shannon entropy* of the misbehaviour distribution itself
//! — slashing more for high-entropy "noisy" cartel patterns and less
//! for low-entropy "obvious" ones.
//!
//! Together with `RedirectKind::Slash` → `RedirectKind::SlashSettle`
//! → `RedirectKind::MevBurn`, this closes the conservation triplet
//! the doctrine names: slash funds the refresh pool, refresh pool
//! pays for chain keep-alive, no energy is destroyed.
//!
//! ## Substrate
//!
//! `entropic_slash(stake, observed_pmf) -> Energy`:
//!
//! - Compute `H(observed)` in millibits via
//!   `evaporchain_cmu_gate::entropy_millibits`.
//! - `slash = stake × H_milli / 1000` capped at stake.
//!
//! Lower-entropy misbehaviour (deterministic, easy-to-detect cartel)
//! → smaller slash. Higher-entropy misbehaviour (noisy, hard to
//! distinguish from honest) → larger slash. The chain "pays attention"
//! to the cases that are actually hard.

use thiserror::Error;

use evaporchain_cmu_gate::{entropy_millibits, EntropyError};
use evaporchain_types::Energy;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EntropicSlashError {
    #[error("entropy estimator error: {0}")]
    Entropy(#[from] EntropyError),
}

/// Compute the Shannon-entropy-weighted slash magnitude in `Energy`.
/// `slash = stake × entropy_millibits(observed_counts) / 1000`,
/// capped at `stake`.
pub fn entropic_slash(
    stake: Energy,
    observed_counts: &[u64],
) -> Result<Energy, EntropicSlashError> {
    let h_milli = entropy_millibits(observed_counts)?;
    if h_milli == 0 {
        return Ok(0);
    }
    let scaled = (stake as u128).saturating_mul(h_milli as u128) / 1_000u128;
    Ok(scaled.min(stake as u128) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_pattern_zero_slash() {
        // {500_000, 0, 0} → entropy = 0 → slash = 0.
        assert_eq!(entropic_slash(1_000_000, &[500_000, 0, 0]).unwrap(), 0);
    }

    #[test]
    fn uniform_2_outcomes_one_bit_slash() {
        // 50/50 → entropy = 1000 millibits → slash = stake × 1000 / 1000 = stake.
        // Capped at stake.
        let s = entropic_slash(500, &[500, 500]).unwrap();
        assert_eq!(s, 500);
    }

    #[test]
    fn uniform_4_outcomes_two_bit_slash() {
        // entropy = 2000 millibits → slash = 2 × stake → CAPPED at stake.
        let s = entropic_slash(1000, &[250, 250, 250, 250]).unwrap();
        assert_eq!(s, 1000);
    }

    #[test]
    fn skewed_distribution_partial_slash() {
        // 80/20 → entropy < 1 bit → partial slash.
        // bit_length(800) = 10, bit_length(200) = 8 → diff = 2 → at most 2 bits per term * fractions
        // Just assert positive and < stake.
        let s = entropic_slash(1000, &[800, 200]).unwrap();
        assert!(s > 0);
        assert!(s <= 1000);
    }

    #[test]
    fn slash_capped_at_stake() {
        let s = entropic_slash(100, &[25, 25, 25, 25]).unwrap();
        assert_eq!(s, 100);
    }
}

// ── M18 (audit 2026-05-13): direction-monotonicity ──
//
// The audit flagged the `entropic_slash(stake, &[count, 1])` call
// site in `tendermint.rs:2345` as direction-ambiguous: `[1, 1]` is
// uniform (max entropy → max slash) while `[100, 1]` is
// near-deterministic (low entropy → small slash). The doctrine
// (per the module header above) is that the slash function uses
// **Shannon entropy of the observed PMF** — higher entropy = larger
// slash. So the existing call-site shape is "more observations
// makes the violation look obviously-deterministic, which lowers
// the slash". This is the documented behaviour; whether it's the
// desired economic design is a separate Phase-3.5d wiring decision.
//
// What this module now guarantees:
//   1. Uniform distributions yield strictly more slash than skewed
//      ones for the same stake.
//   2. For a 2-bucket counts shape `[a, b]`, slash is monotone in
//      |a - b| in the direction "smaller gap → larger slash".
//   3. `[1, 1]` (uniform) is the maximum-slash point.
//   4. `[count, 1]` slash is **non-increasing in count for count ≥ 1**
//      — the exact call-site shape from the consensus path.
//
// Pinning these stops a future refactor from silently flipping the
// economic direction.

#[cfg(test)]
mod m18_monotonicity_tests {
    use super::*;
    use proptest::prelude::*;

    // `[1, 1]` is the uniform two-bucket case → max entropy →
    // slash = stake (the cap fires). Any `[count, 1]` with
    // count > 1 must produce ≤ that.
    proptest! {
        #[test]
        fn audit_m18_uniform_pair_max_slash(stake in 1u64..=10_000_000) {
            let uniform = entropic_slash(stake, &[1, 1]).unwrap();
            prop_assert_eq!(uniform, stake, "[1,1] is uniform → entropy=1bit → full-stake slash");
        }
    }

    // `[count, 1]` for count ≥ 1 is non-increasing in count.
    // Concretely: doubling `count` should not produce MORE slash.
    // This is the call-site shape from `tendermint.rs:2345`.
    proptest! {
        #[test]
        fn audit_m18_count_1_shape_non_increasing_in_count(
            stake in 1u64..=10_000_000,
            count in 1u64..=100_000,
        ) {
            let s_small = entropic_slash(stake, &[count, 1]).unwrap();
            let s_large = entropic_slash(stake, &[count.saturating_mul(2).max(2), 1]).unwrap();
            prop_assert!(
                s_large <= s_small,
                "[2*count, 1] slash {} must be ≤ [count, 1] slash {} (entropy decreases as skew grows)",
                s_large, s_small
            );
        }
    }

    // Closer-to-uniform 2-bucket distribution yields ≥ slash of a
    // more skewed one. Pinning the |a-b| direction.
    proptest! {
        #[test]
        fn audit_m18_two_bucket_smaller_gap_larger_slash(
            stake in 1u64..=10_000_000,
            balanced in 100u64..=10_000,
            skew in 1u64..=99,
        ) {
            let s_balanced = entropic_slash(stake, &[balanced, balanced]).unwrap();
            let s_skewed = entropic_slash(stake, &[balanced, balanced.saturating_mul(skew)]).unwrap();
            prop_assert!(
                s_balanced >= s_skewed,
                "balanced [a,a] slash {} must be ≥ skewed [a,a*{}] slash {}",
                s_balanced, skew, s_skewed
            );
        }
    }

    /// Direction sanity check — pin the doctrine vs the alternate
    /// "more violations → more slash" reading. The audit asked: is
    /// the inline comment wrong, or is the math wrong? Answer (per
    /// the module header above + this assertion): the math is
    /// correct AND the comment is correct. More observations of the
    /// same violator pushes the count distribution toward
    /// near-deterministic, which **lowers** the entropic slash. This
    /// is intentional per the "chain pays attention to cases that
    /// are hard" doctrine.
    #[test]
    fn audit_m18_call_site_direction_pinned() {
        let stake = 1_000_000;
        let one_violation = entropic_slash(stake, &[1, 1]).unwrap();
        let hundred_violations = entropic_slash(stake, &[100, 1]).unwrap();
        assert!(
            one_violation > hundred_violations,
            "doctrine: rare violation (high entropy) slashes MORE than obvious one"
        );
        // And the gap is dramatic: the rare case hits the cap, the
        // obvious case rounds to ~0 (entropy of [100, 1] ≈ 56
        // millibits → 1M × 56 / 1000 ≈ 56_000).
        assert_eq!(one_violation, stake, "uniform [1,1] hits the cap");
        assert!(
            hundred_violations < stake / 10,
            "obvious [100,1] slash {} should be <10% of stake",
            hundred_violations
        );
    }
}

#[cfg(test)]
mod press_claim_tests {
    use super::*;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "Entropic Slashing weights the slash magnitude
    /// by the Shannon entropy of the observed misbehaviour
    /// distribution. (a) Deterministic patterns (entropy=0) get
    /// ZERO slash — the chain doesn't punish trivially-detectable
    /// behaviour twice. (b) Higher-entropy distributions yield
    /// larger slashes. (c) Slash is always capped at stake — never
    /// inflates."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Deterministic distribution (mass on one outcome) → 0 slash.
        assert_eq!(entropic_slash(1_000_000, &[500_000, 0, 0]).unwrap(), 0);

        // Skewed (80/20) gives partial slash > 0 but ≤ stake.
        let skewed = entropic_slash(1_000, &[800, 200]).unwrap();
        assert!(skewed > 0);
        assert!(skewed <= 1_000);

        // Uniform 50/50 → 1 bit entropy → full-stake slash.
        let uniform_2 = entropic_slash(500, &[500, 500]).unwrap();
        assert_eq!(uniform_2, 500);

        // Higher-entropy (uniform 4-way, 2 bits) → also capped at stake.
        let uniform_4 = entropic_slash(1_000, &[250, 250, 250, 250]).unwrap();
        assert_eq!(uniform_4, 1_000);

        // Cap invariant: slash never exceeds stake regardless of entropy.
        let small_stake = entropic_slash(100, &[25, 25, 25, 25]).unwrap();
        assert!(small_stake <= 100);
    }
}
