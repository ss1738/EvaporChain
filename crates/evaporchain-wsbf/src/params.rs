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
