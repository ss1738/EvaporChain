//! Sanov-Slashing helpers — theorem-grade alternative to the
//! hardcoded `0.10` (Equivocation) / `0.01 × missed` (Downtime)
//! percentages currently used in `evaporchain-node`'s slash path.
//!
//! Wraps `evaporchain-sanov-slashing` for the execution-layer caller.
//! The doctrine names Sanov-Slashing as the principled replacement
//! for the existing constants — slash magnitude becomes the *exact*
//! large-deviation cost `KL(observed‖honest)` (Sanov 1957) instead of
//! a hand-tuned rate.
//!
//! Lives alongside the existing slash code; flipping the consumer
//! (`evaporchain-node::main::ConsensusAction::SlashValidator` handler)
//! to use this is a governance amendment, not a code rip-out.
//!
//! ## Honest-baseline distributions
//!
//! Per slash reason, an "honest" target distribution that the chain
//! considers normal validator behaviour:
//!
//! - **Equivocation**: honest = `{committed: 1.0, double-signed: 0.0}`.
//!   An observed double-sign event is a P=0 outcome → KL = ∞ →
//!   `sanov_slash` returns the full stake. Mathematically forces the
//!   maximum slash, matching the spirit of "double-sign = slashed
//!   into the ground" without needing a hardcoded percentage.
//! - **Downtime**: honest = `{produced: 999_000, missed: 1_000}` per
//!   million slots (= 99.9% production rate). The observed
//!   distribution is `{slots − missed, missed}`. KL grows
//!   monotonically with `missed/slots`; `sanov_slash` returns the
//!   cost as a fraction of stake.

use evaporchain_sanov_slashing::{sanov_slash, Distribution, SlashError, FIXED_POINT_SCALE};
use evaporchain_types::Energy;

/// Compute the Sanov slash for an *equivocation* observation.
///
/// The honest distribution puts P(double-sign) = 0; a single observed
/// double-sign violates this with `KL = ∞` → returns `stake`.
pub fn equivocation_slash(stake: Energy) -> Result<Energy, SlashError> {
    let honest = Distribution::new(vec![FIXED_POINT_SCALE, 0])
        .expect("honest equivocation pmf is well-formed by construction");
    let observed = Distribution::new(vec![500_000, 500_000])
        .expect("observed equivocation pmf is well-formed by construction");
    sanov_slash(stake, &observed, &honest)
}

/// Compute the Sanov slash for a *downtime* observation over a
/// `slots`-slot window with `missed` blocks not produced.
///
/// Honest baseline = 99.9% production rate. As `missed/slots` grows,
/// the observed distribution diverges from honest; the slash is
/// proportional to the KL divergence × stake.
///
/// Returns `Ok(0)` for `missed == 0` or `slots == 0`.
pub fn downtime_slash(stake: Energy, slots: u64, missed: u64) -> Result<Energy, SlashError> {
    if slots == 0 || missed == 0 {
        return Ok(0);
    }
    // Honest baseline (per 1_000_000 slots): 999_000 produced / 1_000 missed.
    let honest = Distribution::new(vec![999_000, 1_000])
        .expect("honest downtime pmf is well-formed by construction");
    // Observed pmf from raw counts: produced = slots - missed, missed = missed.
    let produced = slots.saturating_sub(missed);
    let observed = Distribution::from_counts(&[produced, missed]).map_err(|_| {
        SlashError::Kl(evaporchain_sanov_slashing::KlError::AlphabetMismatch { q_len: 2, p_len: 2 })
    })?;
    sanov_slash(stake, &observed, &honest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equivocation_slashes_full_stake() {
        // A double-sign event yields KL = ∞ → full slash.
        let s = equivocation_slash(1_000_000).unwrap();
        assert_eq!(s, 1_000_000);
    }

    #[test]
    fn downtime_zero_missed_no_slash() {
        let s = downtime_slash(1_000_000, 1000, 0).unwrap();
        assert_eq!(s, 0);
    }

    #[test]
    fn downtime_zero_slots_no_slash() {
        let s = downtime_slash(1_000_000, 0, 100).unwrap();
        assert_eq!(s, 0);
    }

    #[test]
    fn downtime_slash_grows_with_missed_count() {
        let stake = 1_000_000;
        let low = downtime_slash(stake, 1000, 5).unwrap();
        let mid = downtime_slash(stake, 1000, 50).unwrap();
        let high = downtime_slash(stake, 1000, 500).unwrap();
        assert!(mid >= low);
        assert!(high >= mid);
    }

    #[test]
    fn downtime_slash_capped_at_stake() {
        let s = downtime_slash(100, 10, 9).unwrap();
        assert!(s <= 100);
    }

    #[test]
    fn at_honest_baseline_minimal_slash() {
        // Observation matching the honest baseline (~0.1% missed)
        // should give a near-zero slash.
        let stake = 1_000_000;
        let s = downtime_slash(stake, 1_000_000, 1_000).unwrap();
        // Zero or close to it (KL of identical distributions is zero).
        assert!(
            s < 100,
            "expected near-zero slash for honest-rate downtime, got {s}"
        );
    }
}
