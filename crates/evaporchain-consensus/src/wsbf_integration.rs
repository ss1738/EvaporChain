//! Wilson-Singh Block Flow integration — wires `evaporchain-wsbf` into
//! the consensus layer.
//!
//! The WSBF RG flow coarse-grains the block history into `EffectiveParams`,
//! renormalizing the chain's effective λ at each step.  High-entropy block
//! windows (complex fee/energy distributions) lower λ_eff, signalling that
//! the chain is operating in a high-complexity regime.
//!
//! # Flow
//!
//! After each committed block: push a `BlockSummary` to the sliding window.
//! When the window reaches `coarse_grain` blocks, run one `rg_step` to
//! produce the latest `EffectiveParams`.  The result is consumed by the
//! RG Phase Map integration to classify the current consensus regime.
//!
//! # Conservation note
//!
//! WSBF only *reads* the block history — it does not modify state.

use std::collections::VecDeque;

use evaporchain_energy_kernel::DEFAULT_LAMBDA;
use evaporchain_wsbf::{
    params::{BlockSummary, EffectiveParams, RgFlowParams},
    rg_step,
};
use tracing::debug;

/// Number of blocks in one WSBF coarse-graining window.
/// Governance-tunable; 100 blocks ≈ one coarse-grained "scale" at
/// typical block rates.
pub const WSBF_COARSE_GRAIN: usize = 100;

/// Entropy rescaling factor in millibits.  Higher → λ_eff drifts down
/// faster in high-entropy windows.
pub const WSBF_ENTROPY_SCALE_MB: u64 = 1_000;

/// Default `RgFlowParams` used by the chain.
pub fn default_rg_params() -> RgFlowParams {
    RgFlowParams {
        coarse_grain: WSBF_COARSE_GRAIN,
        entropy_scale_mb: WSBF_ENTROPY_SCALE_MB,
    }
}

/// Convert a committed block's on-chain data into a `BlockSummary` for the
/// WSBF window.  `tx_count` stands in for `total_energy` (same proxy the
/// LightCone and TUR integrations use at this substrate stage).
pub fn block_to_summary(height: u64, tx_count: u64, active_accounts: u64, epoch: u64) -> BlockSummary {
    let _ = epoch; // reserved for future per-block lambda governance
    BlockSummary {
        height,
        total_energy: tx_count,
        active_accounts,
        lambda_half_life: DEFAULT_LAMBDA.epochs().max(1),
    }
}

/// Push one `BlockSummary` to the sliding window and run `rg_step` when
/// the window is full.  Returns the new `EffectiveParams` on a full window,
/// `None` otherwise.
///
/// The caller maintains the `VecDeque`; this function handles the
/// push-and-drain lifecycle.
pub fn push_and_step(
    window: &mut VecDeque<BlockSummary>,
    summary: BlockSummary,
    params: &RgFlowParams,
) -> Option<EffectiveParams> {
    window.push_back(summary);
    if window.len() < params.coarse_grain {
        return None;
    }
    // Drain the oldest `coarse_grain` entries to form the step window.
    let step_window: Vec<BlockSummary> = window.drain(..params.coarse_grain).collect();
    let step_idx = 0; // step index — always 0 for single-step integration
    match rg_step(&step_window, step_idx, params) {
        Ok(ep) => {
            debug!(
                height_start = ep.height_start,
                height_end   = ep.height_end,
                lambda_eff   = ep.lambda_eff,
                entropy_mb   = ep.entropy_mb,
                "WSBF RG step complete"
            );
            Some(ep)
        }
        Err(e) => {
            debug!(err = %e, "WSBF rg_step failed (best-effort)");
            None
        }
    }
}

/// Convenience wrapper used by `on_block_committed`.
///
/// Builds the summary from raw block data, pushes it, and returns any new
/// `EffectiveParams`.
pub fn on_committed_block(
    window: &mut VecDeque<BlockSummary>,
    height: u64,
    tx_count: u64,
    active_accounts: u64,
    epoch: u64,
    params: &RgFlowParams,
) -> Option<EffectiveParams> {
    let summary = BlockSummary {
        height,
        total_energy: tx_count,
        active_accounts,
        lambda_half_life: DEFAULT_LAMBDA.epochs().max(1),
    };
    let _ = epoch; // reserved for future per-block lambda governance
    push_and_step(window, summary, params)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill_window(n: usize) -> (VecDeque<BlockSummary>, Option<EffectiveParams>) {
        let params = default_rg_params();
        let mut window = VecDeque::new();
        let mut last = None;
        for i in 0..n {
            last = push_and_step(
                &mut window,
                BlockSummary { height: i as u64, total_energy: 50, active_accounts: 10, lambda_half_life: 4096 },
                &params,
            );
        }
        (window, last)
    }

    #[test]
    fn no_output_before_full_window() {
        let (_, ep) = fill_window(WSBF_COARSE_GRAIN - 1);
        assert!(ep.is_none());
    }

    #[test]
    fn outputs_on_full_window() {
        let (_, ep) = fill_window(WSBF_COARSE_GRAIN);
        assert!(ep.is_some());
    }

    #[test]
    fn window_drains_on_step() {
        let (window, _) = fill_window(WSBF_COARSE_GRAIN);
        // After draining coarse_grain entries, window should be empty.
        assert!(window.is_empty());
    }

    #[test]
    fn lambda_eff_at_most_avg_lambda() {
        let (_, ep) = fill_window(WSBF_COARSE_GRAIN);
        let ep = ep.unwrap();
        // λ_eff ≤ avg_λ (entropy correction only decreases it).
        assert!(ep.lambda_eff <= 4096);
    }

    #[test]
    fn zero_energy_window_no_entropy_correction() {
        let params = RgFlowParams { coarse_grain: 4, entropy_scale_mb: 1_000 };
        let mut window = VecDeque::new();
        let mut last = None;
        for i in 0..4 {
            last = push_and_step(
                &mut window,
                BlockSummary { height: i, total_energy: 0, active_accounts: 0, lambda_half_life: 100 },
                &params,
            );
        }
        let ep = last.unwrap();
        // Zero energy → zero entropy → λ_eff = avg_λ = 100.
        assert_eq!(ep.lambda_eff, 100);
    }
}
