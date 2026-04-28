//! `FeeControllerParams` — chain-set parameters for the fee market.
//!
//! Per the doctrine, λ enters via the chain-global `ChainLambda`. The
//! controller does NOT carry its own decay rate — single-λ principle.

use serde::{Deserialize, Serialize};

use evaporchain_energy_kernel::{ChainLambda, Energy};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeeControllerParams {
    /// Target equilibrium energy E*. The integrator-with-leak settles
    /// here when realized gas usage matches `target_gas` on average.
    pub target_energy: Energy,
    /// Target gas usage per block. Equilibrium happens when realized
    /// gas tracks this number on average.
    pub target_gas: u64,
    /// Chain-global λ (single-λ principle).
    pub chain_lambda: ChainLambda,
    /// Controller gain — base-fee response per unit of imbalance,
    /// in parts-per-million. Higher = more aggressive response.
    pub fee_response_ppm: u64,
}

impl FeeControllerParams {
    pub const fn new(
        target_energy: Energy,
        target_gas: u64,
        chain_lambda: ChainLambda,
        fee_response_ppm: u64,
    ) -> Self {
        Self {
            target_energy,
            target_gas,
            chain_lambda,
            fee_response_ppm,
        }
    }

    /// Provisional genesis defaults — pending tokenomics ceremony.
    /// E* = 1_000_000, target_gas = 30_000_000 (≈ Ethereum target),
    /// fee_response_ppm = 125_000 (1/8 in ppm — matches EIP-1559's
    /// natural responsiveness).
    pub fn default_genesis() -> Self {
        Self::new(1_000_000, 30_000_000, ChainLambda::default_genesis(), 125_000)
    }
}

impl Default for FeeControllerParams {
    fn default() -> Self {
        Self::default_genesis()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_genesis_has_sane_values() {
        let p = FeeControllerParams::default_genesis();
        assert_eq!(p.target_energy, 1_000_000);
        assert_eq!(p.target_gas, 30_000_000);
        assert_eq!(p.fee_response_ppm, 125_000);
        assert!(!p.chain_lambda.lambda().is_degenerate());
    }
}
