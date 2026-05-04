//! Sanov-Slashing.
//!
//! Per `research/INVENTION_STACK.md` §A1.3:
//!
//! > **Sanov-Slashing** | Cramér 1938; Sanov 1957 | slash magnitude =
//! > stake × KL-rate function `I(observed ‖ honest)`; replaces ad-hoc
//! > percentages with the *exact* large-deviation cost.
//!
//! Sanov's theorem: the probability that an empirical distribution
//! `Q_n` (over `n` observations) deviates from the truth `P` is
//! approximately `exp(−n · KL(Q_n ‖ P))`. The exponent is the
//! "rate function" `I(Q ‖ P) = KL(Q ‖ P)`. So *the natural cost a
//! validator should pay for misbehaving in a way that looks `Q`-like
//! when honest behaviour is `P`-like* is proportional to `KL(Q ‖ P)` —
//! the very quantity that bounds the probability of seeing such
//! misbehaviour by chance.
//!
//! ## Why this is novel at L1
//!
//! Existing chains hard-code percentages (Cosmos: 5% double-sign, 1%
//! downtime; Ethereum: variable but parameter-bounded; EvaporChain
//! pre-Sanov: 10% Equivocation / 1% Downtime per block). All chosen
//! by intuition. Sanov-Slashing replaces every per-incident percentage
//! with a *closed-form* cost driven by the same large-deviation
//! exponent that bounds the Bayesian-posterior probability that the
//! validator was honest. There is no parameter to tune.
//!
//! ## Module map
//!
//! - [`distribution`] — `Distribution` over a finite outcome alphabet
//!   (sums to a fixed-point representation of 1.0).
//! - [`kl`] — integer Kullback-Leibler divergence
//!   `KL(Q ‖ P) = Σ Q_i · log_2(Q_i / P_i)` in millibits.
//! - [`slash`] — `sanov_slash(stake, observed, honest) -> Energy`
//!   plus the `apply_slash` integration with the energy-kernel.

pub mod distribution;
pub mod kl;
pub mod slash;

pub use distribution::{Distribution, DistributionError, FIXED_POINT_SCALE};
pub use kl::{kl_millibits, KlError};
pub use slash::{apply_slash, sanov_slash, SlashError};

#[cfg(test)]
mod press_claim_tests {
    use super::*;
    use evaporchain_energy_kernel::{Compartment, ConservationCheck, EnergyAccumulator};

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "Sanov-Slashing replaces ad-hoc percentages with
    /// the EXACT large-deviation cost: slash = stake × KL(observed‖
    /// honest) capped at stake. Identical distributions → 0 slash.
    /// `apply_slash` redirects stake to SlashedPool — total energy
    /// is PRESERVED across compartments (conservation invariant)."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let honest = Distribution::new(vec![999_000, 1_000]).unwrap();
        let observed_match = Distribution::new(vec![999_000, 1_000]).unwrap();
        let observed_diverge = Distribution::new(vec![500_000, 500_000]).unwrap();

        // Identical → 0 slash.
        assert_eq!(sanov_slash(1_000_000, &observed_match, &honest).unwrap(), 0);

        // Strict divergence → positive slash, capped at stake.
        let s = sanov_slash(1_000_000, &observed_diverge, &honest).unwrap();
        assert!(s > 0);
        assert!(s <= 1_000_000);

        // Impossible-event (KL=+∞) → full-stake slash.
        let p_impossible = Distribution::new(vec![FIXED_POINT_SCALE, 0]).unwrap();
        let q_violation = Distribution::new(vec![900_000, 100_000]).unwrap();
        assert_eq!(
            sanov_slash(500_000, &q_violation, &p_impossible).unwrap(),
            500_000
        );

        // apply_slash conserves total energy (Stake → SlashedPool).
        let mut acc = EnergyAccumulator::new(0, 1_000_000, 0, 0);
        let before = acc;
        let amt = apply_slash(1_000_000, &observed_diverge, &honest, &mut acc).unwrap();
        assert!(amt > 0);
        assert_eq!(acc[Compartment::Stake], 1_000_000 - amt);
        assert_eq!(acc[Compartment::SlashedPool], amt);
        // Conservation invariant: total preserved.
        ConservationCheck::redirect(&before, &acc).expect("slash must conserve total");

        // Insufficient-stake apply_slash fails closed without mutation.
        let mut acc_short = EnergyAccumulator::new(0, 100, 0, 0);
        let pre = acc_short;
        let err = apply_slash(1_000_000, &q_violation, &p_impossible, &mut acc_short)
            .unwrap_err();
        assert!(matches!(err, SlashError::StakeBelowSlash { .. }));
        assert_eq!(acc_short, pre);
    }
}
