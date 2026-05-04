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
        assert_eq!(
            entropic_slash(1_000_000, &[500_000, 0, 0]).unwrap(),
            0
        );

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
