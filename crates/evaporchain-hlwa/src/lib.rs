//! Half-Life Wrapped Asset (HLWA) — Tier 2.
//!
//! Per `research/INVENTION_STACK.md` §4.2:
//!
//! > **Half-Life Wrapped Asset (HLWA)** — Wrapped tokens decay unless
//! > re-attested by origin chain — eliminates infinite-bridge-supply
//! > hacks.
//!
//! ## Why
//!
//! Ronin / Wormhole / Nomad were all "infinite supply" bridge hacks:
//! the bridge contract minted wrapped tokens whose total supply
//! drifted away from the origin-chain backing. HLWA makes that
//! attack impossible by *automatically reducing* the wrapped supply
//! whenever the origin attestation goes stale — a hostile bridge
//! operator who stops attesting watches their supply evaporate
//! instead of staying inflated.
//!
//! ## Mechanics
//!
//! Each `WrappedAsset` carries:
//!
//! - `current_supply` — the live wrapped balance.
//! - `origin_attested_supply` — the supply the origin chain has
//!   actually attested in the last refresh.
//! - `last_attested_epoch` — when the latest origin attestation arrived.
//! - `attestation_lambda` — half-life of the attestation freshness.
//!
//! `decay(current_epoch)` returns the *effective* supply at
//! `current_epoch` — `origin_attested_supply` decayed under
//! `attestation_lambda`. The chain's burn-on-stale-attestation logic
//! reads this value and burns excess wrapped supply down to it.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use evaporchain_energy_kernel::{energy_at_epoch, ChainLambda};
use evaporchain_types::Energy;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WrappedAsset {
    pub current_supply: Energy,
    pub origin_attested_supply: Energy,
    pub last_attested_epoch: u64,
    pub attestation_lambda: ChainLambda,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum HlwaError {
    #[error("attestation epoch {attestation} is in the future relative to {current}")]
    AttestationFromFuture { attestation: u64, current: u64 },
}

impl WrappedAsset {
    pub const fn new(
        current_supply: Energy,
        origin_attested_supply: Energy,
        last_attested_epoch: u64,
        attestation_lambda: ChainLambda,
    ) -> Self {
        Self {
            current_supply,
            origin_attested_supply,
            last_attested_epoch,
            attestation_lambda,
        }
    }

    /// Effective supply at `current_epoch` = `origin_attested_supply`
    /// decayed under `attestation_lambda` from `last_attested_epoch`.
    /// This is the ceiling the chain's burn-on-stale logic reads.
    pub fn effective_supply(&self, current_epoch: u64) -> Result<Energy, HlwaError> {
        if current_epoch < self.last_attested_epoch {
            return Err(HlwaError::AttestationFromFuture {
                attestation: self.last_attested_epoch,
                current: current_epoch,
            });
        }
        let elapsed = current_epoch - self.last_attested_epoch;
        Ok(energy_at_epoch(
            self.origin_attested_supply,
            self.attestation_lambda.half_life(),
            elapsed,
        ))
    }

    /// Refresh from a new origin attestation — sets the attested
    /// supply + epoch. Caller is responsible for verifying the
    /// attestation cryptographically before invoking.
    pub fn re_attest(mut self, attested_supply: Energy, epoch: u64) -> Self {
        self.origin_attested_supply = attested_supply;
        self.last_attested_epoch = epoch;
        self
    }

    /// Compute how much wrapped supply must be burnt to bring
    /// `current_supply` down to `effective_supply(current_epoch)`.
    /// Returns 0 if already at-or-below the ceiling.
    pub fn excess_to_burn(&self, current_epoch: u64) -> Result<Energy, HlwaError> {
        let ceiling = self.effective_supply(current_epoch)?;
        Ok(self.current_supply.saturating_sub(ceiling))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_energy_kernel::Lambda;

    fn lambda() -> ChainLambda {
        ChainLambda::new(Lambda::from_epochs(100))
    }

    #[test]
    fn effective_at_attestation_is_attested_supply() {
        let a = WrappedAsset::new(1000, 1000, 5, lambda());
        assert_eq!(a.effective_supply(5).unwrap(), 1000);
    }

    #[test]
    fn effective_decays_with_stale_attestation() {
        let a = WrappedAsset::new(1000, 1000, 0, lambda());
        // After one half-life, ceiling halves.
        assert_eq!(a.effective_supply(100).unwrap(), 500);
    }

    #[test]
    fn excess_to_burn_when_supply_above_ceiling() {
        let a = WrappedAsset::new(1000, 1000, 0, lambda());
        // After 100 epochs, ceiling = 500; current = 1000; excess = 500.
        assert_eq!(a.excess_to_burn(100).unwrap(), 500);
    }

    #[test]
    fn no_excess_when_at_ceiling() {
        let a = WrappedAsset::new(500, 1000, 0, lambda());
        // After 100 epochs, ceiling = 500; current already at 500 → no excess.
        assert_eq!(a.excess_to_burn(100).unwrap(), 0);
    }

    #[test]
    fn re_attest_resets_ceiling() {
        let a = WrappedAsset::new(1000, 1000, 0, lambda());
        let refreshed = a.re_attest(1500, 200);
        assert_eq!(refreshed.origin_attested_supply, 1500);
        assert_eq!(refreshed.last_attested_epoch, 200);
        // At epoch 200, ceiling = full attested = 1500.
        assert_eq!(refreshed.effective_supply(200).unwrap(), 1500);
    }

    #[test]
    fn future_attestation_rejected() {
        let a = WrappedAsset::new(1000, 1000, 100, lambda());
        let err = a.effective_supply(50).unwrap_err();
        assert!(matches!(err, HlwaError::AttestationFromFuture { .. }));
    }
}
