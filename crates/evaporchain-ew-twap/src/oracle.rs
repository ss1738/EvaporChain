//! `EwTwapOracle` — the energy-weighted TWAP oracle.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::obs::{Observation, PriceMicros};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum OracleError {
    #[error("non-monotone observation: incoming epoch {incoming} ≤ last logged {last}")]
    NonMonotoneEpoch { incoming: u64, last: u64 },
    #[error("no observations within window — oracle has no signal to read")]
    NoObservations,
    #[error("zero total energy across the window — oracle reading would divide by zero")]
    ZeroTotalEnergy,
    #[error("price·energy product overflowed u128")]
    Overflow,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EwTwapOracle {
    window_epochs: u64,
    observations: VecDeque<Observation>,
}

impl EwTwapOracle {
    /// New empty oracle with given window.
    ///
    /// `window_epochs` of 0 means "no window — keep all observations
    /// indefinitely". Practical deployments will use a non-zero
    /// window matching the application's manipulation-resistance
    /// horizon.
    pub fn new(window_epochs: u64) -> Self {
        Self {
            window_epochs,
            observations: VecDeque::new(),
        }
    }

    pub fn window_epochs(&self) -> u64 {
        self.window_epochs
    }

    pub fn observation_count(&self) -> usize {
        self.observations.len()
    }

    pub fn observations(&self) -> impl Iterator<Item = &Observation> {
        self.observations.iter()
    }

    /// Append a new observation. Enforces strictly monotone epoch
    /// ordering and prunes stale observations as a side effect.
    pub fn observe(&mut self, obs: Observation) -> Result<(), OracleError> {
        if let Some(last) = self.observations.back() {
            if obs.epoch <= last.epoch {
                return Err(OracleError::NonMonotoneEpoch {
                    incoming: obs.epoch,
                    last: last.epoch,
                });
            }
        }
        self.observations.push_back(obs);
        self.prune_at(obs.epoch);
        Ok(())
    }

    /// Drop observations whose epoch is more than `window_epochs`
    /// old relative to `now`.
    pub fn prune_at(&mut self, now: u64) {
        if self.window_epochs == 0 {
            return;
        }
        let cutoff = now.saturating_sub(self.window_epochs);
        while let Some(o) = self.observations.front() {
            if o.epoch < cutoff {
                self.observations.pop_front();
            } else {
                break;
            }
        }
    }

    /// Compute the EW-TWAP. Errors if no observations within window
    /// or if the total energy is 0.
    pub fn read(&self) -> Result<PriceMicros, OracleError> {
        if self.observations.is_empty() {
            return Err(OracleError::NoObservations);
        }
        let mut sum_price_x_energy: u128 = 0;
        let mut sum_energy: u128 = 0;
        for o in &self.observations {
            let prod = (o.price_micros as u128)
                .checked_mul(o.energy as u128)
                .ok_or(OracleError::Overflow)?;
            sum_price_x_energy = sum_price_x_energy
                .checked_add(prod)
                .ok_or(OracleError::Overflow)?;
            sum_energy = sum_energy
                .checked_add(o.energy as u128)
                .ok_or(OracleError::Overflow)?;
        }
        if sum_energy == 0 {
            return Err(OracleError::ZeroTotalEnergy);
        }
        let result = sum_price_x_energy / sum_energy;
        // Safe: each price_micros fits u64; the weighted mean is
        // bounded above by the maximum observed price.
        Ok(result as u64)
    }

    /// Compute the EW-TWAP at a specific `now` epoch — applies
    /// window pruning relative to `now` first. Useful when the
    /// chain advances to `now` without a fresh observation; this
    /// gives the *current* EW-TWAP based on still-in-window
    /// observations.
    pub fn read_at(&self, now: u64) -> Result<PriceMicros, OracleError> {
        if self.window_epochs == 0 {
            return self.read();
        }
        let cutoff = now.saturating_sub(self.window_epochs);
        let mut sum_price_x_energy: u128 = 0;
        let mut sum_energy: u128 = 0;
        let mut found_any = false;
        for o in &self.observations {
            if o.epoch < cutoff {
                continue;
            }
            found_any = true;
            let prod = (o.price_micros as u128)
                .checked_mul(o.energy as u128)
                .ok_or(OracleError::Overflow)?;
            sum_price_x_energy = sum_price_x_energy
                .checked_add(prod)
                .ok_or(OracleError::Overflow)?;
            sum_energy = sum_energy
                .checked_add(o.energy as u128)
                .ok_or(OracleError::Overflow)?;
        }
        if !found_any {
            return Err(OracleError::NoObservations);
        }
        if sum_energy == 0 {
            return Err(OracleError::ZeroTotalEnergy);
        }
        Ok((sum_price_x_energy / sum_energy) as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(epoch: u64, price: u64, energy: u64) -> Observation {
        Observation::new(epoch, price, energy)
    }

    // ── basic observe + read ──────────────────────────────────────

    #[test]
    fn empty_oracle_errors_on_read() {
        let o: EwTwapOracle = EwTwapOracle::new(100);
        assert_eq!(o.read().unwrap_err(), OracleError::NoObservations);
    }

    #[test]
    fn single_observation_returns_its_price() {
        let mut o = EwTwapOracle::new(100);
        o.observe(obs(10, 1_500_000, 1000)).unwrap();
        assert_eq!(o.read().unwrap(), 1_500_000);
    }

    #[test]
    fn equal_energy_gives_arithmetic_mean() {
        // Two observations at equal energy: the EW-TWAP is the
        // unweighted mean.
        let mut o = EwTwapOracle::new(100);
        o.observe(obs(10, 1_000_000, 1000)).unwrap();
        o.observe(obs(11, 2_000_000, 1000)).unwrap();
        assert_eq!(o.read().unwrap(), 1_500_000);
    }

    // ── EW-TWAP differs from unweighted ──────────────────────────

    #[test]
    fn higher_energy_observation_dominates() {
        // A high-energy fresh observation outweighs a low-energy
        // stale one even though both are within the window.
        let mut o = EwTwapOracle::new(100);
        // Stale: low energy.
        o.observe(obs(10, 1_000_000, 100)).unwrap();
        // Fresh: high energy.
        o.observe(obs(11, 2_000_000, 10_000)).unwrap();
        // Weighted mean: (1_000_000·100 + 2_000_000·10_000) / 10_100
        //              ≈ 1_990_099
        let r = o.read().unwrap();
        assert!(r > 1_990_000 && r < 2_000_000, "got {r}");
    }

    #[test]
    fn manipulation_via_low_energy_attestations_dampened() {
        // Adversary submits 100 stale-energy attestations of a
        // wildly off-market price; one honest fresh observation
        // dominates.
        let mut o = EwTwapOracle::new(10_000);
        for i in 0..100 {
            // Adversary: price 10_000_000 (10x), low energy 1.
            o.observe(obs(i + 1, 10_000_000, 1)).unwrap();
        }
        // One honest observation with realistic energy.
        o.observe(obs(101, 1_000_000, 100_000)).unwrap();
        // Total adversary weight: 100·1 = 100 (price·energy = 10⁹).
        // Total honest weight: 100_000 (price·energy = 10¹¹).
        // EW-TWAP ≈ (10⁹ + 10¹¹) / (100 + 100_000) ≈ 1_009_990
        let r = o.read().unwrap();
        assert!(r < 1_100_000, "manipulation not dampened: got {r}");
    }

    // ── monotone epoch ───────────────────────────────────────────

    #[test]
    fn non_monotone_epoch_rejected() {
        let mut o = EwTwapOracle::new(100);
        o.observe(obs(10, 1_000_000, 1000)).unwrap();
        let err = o.observe(obs(10, 2_000_000, 1000)).unwrap_err();
        assert_eq!(
            err,
            OracleError::NonMonotoneEpoch {
                incoming: 10,
                last: 10
            }
        );
        let err = o.observe(obs(5, 2_000_000, 1000)).unwrap_err();
        assert!(matches!(err, OracleError::NonMonotoneEpoch { .. }));
    }

    // ── window eviction ──────────────────────────────────────────

    #[test]
    fn observation_outside_window_evicted_on_next_observe() {
        let mut o = EwTwapOracle::new(10);
        o.observe(obs(0, 1_000_000, 1000)).unwrap(); // very old
        o.observe(obs(20, 2_000_000, 1000)).unwrap(); // 20 - 10 = cutoff at 10; epoch 0 evicted
        assert_eq!(o.observation_count(), 1);
        assert_eq!(o.read().unwrap(), 2_000_000);
    }

    #[test]
    fn window_zero_keeps_everything() {
        let mut o = EwTwapOracle::new(0);
        o.observe(obs(0, 1_000_000, 1000)).unwrap();
        o.observe(obs(1_000_000, 2_000_000, 1000)).unwrap();
        assert_eq!(o.observation_count(), 2);
    }

    #[test]
    fn read_at_advances_window_without_new_observation() {
        let mut o = EwTwapOracle::new(10);
        o.observe(obs(5, 1_000_000, 1000)).unwrap();
        // Read at epoch 12: cutoff = 12 - 10 = 2, so obs(5, ...)
        // is in window. Returns 1_000_000.
        assert_eq!(o.read_at(12).unwrap(), 1_000_000);
        // Read at epoch 100: cutoff = 90, obs(5, ...) is OUT of
        // window. NoObservations.
        let err = o.read_at(100).unwrap_err();
        assert_eq!(err, OracleError::NoObservations);
    }

    // ── overflow guard ───────────────────────────────────────────

    #[test]
    fn overflow_in_price_x_energy_surfaces() {
        let mut o = EwTwapOracle::new(0);
        // u64::MAX as price; u64::MAX as energy → product overflows
        // even u128 (no, u128 holds it: 2^64 · 2^64 = 2^128 — fits
        // exactly at upper bound). The accumulator's add will
        // overflow if multiple such observations stack.
        o.observe(obs(1, u64::MAX, u64::MAX)).unwrap();
        // Single one: product = (2^64 - 1)² < 2^128, fits. Reads OK.
        assert_eq!(o.read().unwrap(), u64::MAX);
        // Add another: sum_price_x_energy overflows u128.
        o.observe(obs(2, u64::MAX, u64::MAX)).unwrap();
        let err = o.read().unwrap_err();
        assert_eq!(err, OracleError::Overflow);
    }

    // ── validator-determinism ────────────────────────────────────

    #[test]
    fn read_is_deterministic() {
        let mut o = EwTwapOracle::new(100);
        for i in 1..=5 {
            o.observe(obs(i, 1_000_000 + i * 100_000, 1000 + i * 100))
                .unwrap();
        }
        let r1 = o.read().unwrap();
        let r2 = o.read().unwrap();
        assert_eq!(r1, r2);
    }

    #[test]
    fn read_at_is_deterministic() {
        let mut o = EwTwapOracle::new(100);
        for i in 1..=5 {
            o.observe(obs(i, 1_000_000 + i * 100_000, 1000 + i * 100))
                .unwrap();
        }
        let r1 = o.read_at(50).unwrap();
        let r2 = o.read_at(50).unwrap();
        assert_eq!(r1, r2);
    }

    // ── doctrine claim ────────────────────────────────────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "EW-TWAP is the first oracle whose manipulation
        // resistance scales with energy. Sustaining a manipulated
        // price requires re-attesting it with fresh energy every
        // epoch, because old attestations' energy decays toward
        // floor. The chain's single-λ binds manipulation cost to
        // decay rate."
        //
        // Demonstrate: a sequence of observations where the
        // 'attacker' submits high-volume but low-per-attestation
        // energy attestations gets dampened by a single high-
        // energy honest attestation.
        let mut o = EwTwapOracle::new(10_000);

        // Attacker phase: 50 epochs of attempted manipulation
        // each with energy 10. Total: 500.
        let attacker_price = 5_000_000;
        for i in 1..=50 {
            o.observe(obs(i, attacker_price, 10)).unwrap();
        }

        // Honest phase: ONE attestation with energy 100_000 — i.e.,
        // 200x the attacker's combined weight.
        let honest_price = 1_000_000;
        o.observe(obs(51, honest_price, 100_000)).unwrap();

        let r = o.read().unwrap();
        // The honest observation dominates; the EW-TWAP is much
        // closer to the honest price than the attacker price.
        let dist_to_honest = if r >= honest_price {
            r - honest_price
        } else {
            honest_price - r
        };
        let dist_to_attacker = if r >= attacker_price {
            r - attacker_price
        } else {
            attacker_price - r
        };
        assert!(
            dist_to_honest < dist_to_attacker,
            "EW-TWAP should be closer to honest"
        );
    }

    /// T1.20 — adversarial: when ALL observations have energy 0,
    /// `read()` surfaces `ZeroTotalEnergy` rather than dividing
    /// by zero (lines 105-107). Pin the error path that the
    /// `sum_energy == 0` gate protects.
    #[test]
    fn t1_20_zero_total_energy_error_when_all_energy_zero() {
        let mut o = EwTwapOracle::new(100);
        // Both observations have legitimate prices but zero energy.
        o.observe(obs(10, 1_000_000, 0)).unwrap();
        o.observe(obs(11, 2_000_000, 0)).unwrap();
        assert_eq!(o.read().unwrap_err(), OracleError::ZeroTotalEnergy);
        // read_at is symmetric.
        assert_eq!(o.read_at(12).unwrap_err(), OracleError::ZeroTotalEnergy);
    }

    /// T1.20 — `prune_at(now)` is callable directly (not just as
    /// a side effect of `observe`). Pin that the standalone API
    /// drops stale entries and is idempotent. Useful when the
    /// chain advances epoch without a new observation.
    #[test]
    fn t1_20_prune_at_directly_drops_stale_entries() {
        let mut o = EwTwapOracle::new(10);
        o.observe(obs(5, 1_000_000, 1000)).unwrap();
        o.observe(obs(10, 2_000_000, 1000)).unwrap();
        assert_eq!(o.observation_count(), 2);
        // Prune at epoch 20: cutoff = 10, so obs(5) is dropped.
        o.prune_at(20);
        assert_eq!(o.observation_count(), 1);
        // Idempotent: re-pruning at same epoch is a no-op.
        o.prune_at(20);
        assert_eq!(o.observation_count(), 1);
        // Prune further: cutoff = 50, obs(10) dropped too.
        o.prune_at(60);
        assert_eq!(o.observation_count(), 0);
    }

    /// T1.20 — `window_epochs == 0` means "no window — keep
    /// everything". `read_at` must short-circuit to `read()` in
    /// this case (line 121), so `now` is ignored.
    #[test]
    fn t1_20_read_at_with_zero_window_uses_read() {
        let mut o = EwTwapOracle::new(0);
        o.observe(obs(5, 1_000_000, 1000)).unwrap();
        o.observe(obs(1_000_000, 2_000_000, 1000)).unwrap();
        // window=0 → read_at ignores `now` and uses all observations.
        let r_at_1 = o.read_at(1).unwrap();
        let r_at_huge = o.read_at(u64::MAX).unwrap();
        let r_plain = o.read().unwrap();
        assert_eq!(r_at_1, r_plain);
        assert_eq!(r_at_huge, r_plain);
        // Equal energy → arithmetic mean = 1_500_000.
        assert_eq!(r_plain, 1_500_000);
    }

    proptest::proptest! {
        #[test]
        fn property_ew_twap_in_observed_range(
            n in 1usize..20usize,
            price_lo in 1_000u64..10_000_000u64,
            price_hi_extra in 0u64..10_000_000u64,
            seed in 0u64..1000u64,
        ) {
            // The EW-TWAP must always lie within [min observed
            // price, max observed price]. Property of any convex
            // combination.
            let price_hi = price_lo.saturating_add(price_hi_extra);
            let mut o = EwTwapOracle::new(0);
            let mut min_p = u64::MAX;
            let mut max_p = 0u64;
            for i in 0..n {
                // Mix prices using the seed.
                let p = if (seed.wrapping_mul(31).wrapping_add(i as u64)) % 2 == 0 {
                    price_lo
                } else {
                    price_hi
                };
                let e = (i as u64 + 1) * 100;
                o.observe(obs((i + 1) as u64, p, e)).unwrap();
                min_p = min_p.min(p);
                max_p = max_p.max(p);
            }
            let r = o.read().unwrap();
            proptest::prop_assert!(r >= min_p && r <= max_p, "EW-TWAP {r} out of [{min_p}, {max_p}]");
        }

        #[test]
        fn property_doubling_all_energies_does_not_change_ew_twap(
            n in 1usize..16usize,
            seed in 0u64..1000u64,
        ) {
            // Multiplying every observation's energy by a constant
            // doesn't change the weighted mean.
            let mut o1 = EwTwapOracle::new(0);
            let mut o2 = EwTwapOracle::new(0);
            for i in 0..n {
                let p = 1_000_000 + (seed.wrapping_mul(7).wrapping_add(i as u64)) % 5_000_000;
                let e = 100 + (seed.wrapping_mul(11).wrapping_add(i as u64)) % 10_000;
                o1.observe(obs((i + 1) as u64, p, e)).unwrap();
                o2.observe(obs((i + 1) as u64, p, e * 2)).unwrap();
            }
            let r1 = o1.read().unwrap();
            let r2 = o2.read().unwrap();
            proptest::prop_assert_eq!(r1, r2);
        }
    }
}
