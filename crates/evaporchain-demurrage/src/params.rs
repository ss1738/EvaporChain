//! `DemurrageParams` — chain-set parameters for the piecewise-log rate.
//!
//! Both fields are governance-rotatable; the genesis defaults are
//! conservative placeholders pending the tokenomics ceremony per
//! INVENTION_STACK.md §8 open question "Total chain energy budget at
//! genesis sets all other constants."

use serde::{Deserialize, Serialize};

use evaporchain_types::Energy;

/// Chain-set parameters governing the demurrage rate.
///
/// `lambda_base_ppm` is the per-epoch rate coefficient in parts-per-
/// million. The actual rate at a given balance is
/// `lambda_base_ppm · log_2(balance / threshold)` ppm of `balance` per
/// epoch — see [`crate::rate::rate_ppm`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DemurrageParams {
    /// λ_base in parts-per-million per epoch. Range governance-bounded
    /// in production; `1` is a meaningfully gentle rate (1 ppm per
    /// epoch per log2-doubling above threshold).
    pub lambda_base_ppm: u64,
    /// Energy threshold below which demurrage is zero.
    pub threshold: Energy,
}

impl DemurrageParams {
    pub const fn new(lambda_base_ppm: u64, threshold: Energy) -> Self {
        Self {
            lambda_base_ppm,
            threshold,
        }
    }

    /// Genesis defaults: 1 ppm/epoch/log₂-doubling above the threshold.
    ///
    /// **Threshold = 250_000 EVP (testnet calibration).** Validators on
    /// the running 5-node WAN cluster sit at 300k–600k EVP, so a 250k
    /// threshold means the layer-5 decay-thesis is *empirically observable*
    /// in production: validators slightly above threshold pay ~0.1–1
    /// EVP/epoch in demurrage, well below their per-block reward income.
    ///
    /// Two earlier calibrations and why this one:
    /// - `threshold=1024`  (V0): too aggressive — 500k-EVP validators
    ///   lost ~189k EVP in 8 hours (V1 gas-broke incident 2026-05-08).
    /// - `threshold=100_000_000` (mainnet calibration): too lax for the
    ///   testnet — all validators sit far below 100M, so demurrage is
    ///   parameter-gated dormant. The substrate fires correctly but
    ///   produces zero observable burn (see
    ///   `AUDIT_2026_05_08_DECAY_LOOP.md` addendum). Reserve for mainnet
    ///   genesis where Foundation Treasury (350M EVP) and large
    ///   institutional validators exceed the threshold.
    /// - `threshold=250_000` (this commit, testnet): goldilocks. ≤250k
    ///   validators still get zero (ratchet-4 "death is final" doctrine
    ///   handles them via storage-rent → tombstone instead). Above-
    ///   threshold validators see ~0.1–1 EVP/epoch decay → empirical
    ///   evidence of layer-5 in action without V0-style validator
    ///   collapse.
    ///
    /// Math check at 600k EVP: log₂(600k/250k) ≈ 1.26; rate ≈ 1.26 ppm;
    /// burn per epoch ≈ 0.76 EVP. At ~1 block/sec that's ~65k EVP/day —
    /// well below the ~1.7M EVP/day in block rewards a typical 1-of-5
    /// validator earns. Layer-5 fires visibly; validators stay solvent.
    pub const fn default_genesis() -> Self {
        Self::new(1, 250_000)
    }

    /// Mainnet calibration — matches the original genesis comment with
    /// the 100M threshold designed for a chain whose Foundation
    /// Treasury holds 350M EVP and validators are funded at 50M+.
    /// Reserved for mainnet genesis; testnet uses `default_genesis()`
    /// which has a 250k threshold so the mechanism is observable on
    /// realistic-testnet-scale validator balances.
    pub const fn mainnet_calibration() -> Self {
        Self::new(1, 100_000_000)
    }

    /// Convenience: a "no demurrage" param set (rate coefficient = 0).
    /// Useful for tests and for chains that want to disable demurrage
    /// at the parameter level without removing the code path.
    pub const fn disabled() -> Self {
        Self::new(0, u64::MAX)
    }

    /// Is this parameter set effectively a no-op?
    pub const fn is_disabled(&self) -> bool {
        self.lambda_base_ppm == 0
    }
}

impl Default for DemurrageParams {
    fn default() -> Self {
        Self::default_genesis()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_genesis_non_disabled() {
        // Testnet calibration — 250k threshold so the mechanism is
        // observable on the running cluster where validators are at
        // 300k-600k EVP. Mainnet genesis would use the 100M threshold
        // via `mainnet_calibration()`.
        let p = DemurrageParams::default_genesis();
        assert!(!p.is_disabled());
        assert_eq!(p.lambda_base_ppm, 1);
        assert_eq!(p.threshold, 250_000);
    }

    #[test]
    fn mainnet_calibration_matches_doctrine_comment() {
        // The original 100M threshold is preserved as a separate
        // constructor for future mainnet genesis. This test pins the
        // value so a refactor can't silently drift it.
        let p = DemurrageParams::mainnet_calibration();
        assert_eq!(p.lambda_base_ppm, 1);
        assert_eq!(p.threshold, 100_000_000);
    }

    #[test]
    fn disabled_round_trip() {
        let p = DemurrageParams::disabled();
        assert!(p.is_disabled());
    }

    #[test]
    fn default_matches_default_genesis() {
        assert_eq!(
            DemurrageParams::default(),
            DemurrageParams::default_genesis()
        );
    }
}
