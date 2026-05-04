//! Data types for the WSBF RG flow.

use serde::{Deserialize, Serialize};

/// A coarse summary of one block's energy state — the "field configuration"
/// fed into the RG integrator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockSummary {
    /// Block height.
    pub height: u64,
    /// Sum of all account energies in this block (the "action density").
    pub total_energy: u64,
    /// Number of active accounts (objects with energy > 0).
    pub active_accounts: u64,
    /// λ half-life at this block.
    pub lambda_half_life: u64,
}

/// Parameters controlling one RG step.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct RgFlowParams {
    /// Number of blocks to integrate over per RG step (the "momentum shell width").
    pub coarse_grain: usize,
    /// Entropy rescaling factor in millibits (controls λ_eff drift).
    /// Higher values mean the entropy correction to λ_eff is larger.
    pub entropy_scale_mb: u64,
}

impl Default for RgFlowParams {
    fn default() -> Self {
        Self {
            coarse_grain: 100,
            entropy_scale_mb: 1_000,
        }
    }
}

/// The "effective Lagrangian" after integrating out one block window.
/// Carries the renormalized λ and a coarse account-energy density.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectiveParams {
    /// RG step index (0 = physical, 1 = first coarse-graining, …).
    pub step: usize,
    /// Block height range this step covers.
    pub height_start: u64,
    pub height_end: u64,
    /// Renormalized λ after this step.
    /// `λ_eff = weighted_avg(λ_i) − entropy_correction`
    pub lambda_eff: u64,
    /// Effective account count (coarse-grained from active_accounts).
    pub effective_accounts: u64,
    /// Effective energy density: total_energy / coarse_grain blocks.
    pub energy_density: u64,
    /// Shannon entropy (in millibits) of the energy distribution in this window.
    pub entropy_mb: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rg_flow_params_default_is_documented_baseline() {
        let p = RgFlowParams::default();
        assert_eq!(p.coarse_grain, 100);
        assert_eq!(p.entropy_scale_mb, 1_000);
    }

    #[test]
    fn block_summary_serde_round_trip() {
        let bs = BlockSummary {
            height: 42,
            total_energy: 1_000_000,
            active_accounts: 128,
            lambda_half_life: 4096,
        };
        let json = serde_json::to_string(&bs).unwrap();
        let back: BlockSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(bs.height, back.height);
        assert_eq!(bs.total_energy, back.total_energy);
        assert_eq!(bs.active_accounts, back.active_accounts);
        assert_eq!(bs.lambda_half_life, back.lambda_half_life);
    }

    #[test]
    fn effective_params_serde_round_trip() {
        let ep = EffectiveParams {
            step: 1,
            height_start: 0,
            height_end: 99,
            lambda_eff: 4000,
            effective_accounts: 100,
            energy_density: 500,
            entropy_mb: 750,
        };
        let json = serde_json::to_string(&ep).unwrap();
        let back: EffectiveParams = serde_json::from_str(&json).unwrap();
        assert_eq!(ep.step, back.step);
        assert_eq!(ep.lambda_eff, back.lambda_eff);
        assert_eq!(ep.entropy_mb, back.entropy_mb);
    }

    #[test]
    fn rg_flow_params_is_copy() {
        // Ensure RgFlowParams stays Copy — the RG inner loop relies on
        // it being cheap to pass by value through repeated ticks.
        let p = RgFlowParams::default();
        let _q = p; // would not compile if Copy were removed
        let _r = p;
    }
}
