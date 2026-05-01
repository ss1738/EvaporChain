//! Core RG integration step.
//!
//! One WSBF RG step:
//!   1. Collect `coarse_grain` consecutive block summaries.
//!   2. Compute the effective energy density, account count, and Shannon entropy.
//!   3. Renormalize λ: `λ_eff = avg_λ − entropy_correction`.
//!      The entropy correction captures how much "information" (complexity) is
//!      being integrated out — higher entropy windows shift λ_eff down faster,
//!      corresponding to faster effective decay in a high-complexity regime.
//!   4. Return the `EffectiveParams` for this RG level.

use thiserror::Error;

use crate::params::{BlockSummary, EffectiveParams, RgFlowParams};

#[derive(Debug, Error)]
pub enum RgFlowError {
    #[error("window is empty — need at least 1 block to coarse-grain")]
    EmptyWindow,
    #[error("window length {0} does not match coarse_grain {1}")]
    WindowLengthMismatch(usize, usize),
}

/// Apply one RG step to a block window of length `params.coarse_grain`.
///
/// Returns the `EffectiveParams` representing the integrated-out window.
pub fn rg_step(
    window: &[BlockSummary],
    step: usize,
    params: &RgFlowParams,
) -> Result<EffectiveParams, RgFlowError> {
    if window.is_empty() {
        return Err(RgFlowError::EmptyWindow);
    }
    if window.len() != params.coarse_grain {
        return Err(RgFlowError::WindowLengthMismatch(
            window.len(),
            params.coarse_grain,
        ));
    }

    let n = window.len() as u64;
    let total_energy: u64 = window
        .iter()
        .map(|b| b.total_energy)
        .fold(0, u64::saturating_add);
    let total_accounts: u64 = window
        .iter()
        .map(|b| b.active_accounts)
        .fold(0, u64::saturating_add);
    let avg_lambda: u64 = window
        .iter()
        .map(|b| b.lambda_half_life)
        .fold(0u128, |a, l| a + l as u128) as u64
        / n;

    let energy_density = total_energy / n;
    let effective_accounts = total_accounts / n;

    // Shannon entropy of the per-block energy distribution in millibits.
    // H = −sum_i p_i × log2(p_i) where p_i = total_energy_i / total_energy.
    let entropy_mb = if total_energy == 0 {
        0
    } else {
        window
            .iter()
            .filter(|b| b.total_energy > 0)
            .map(|b| {
                // p_i = b.total_energy / total_energy (rational, no float).
                // log2(p_i) approximated by bit_length(p_i) − bit_length(total_energy).
                let p_num = b.total_energy;
                let p_den = total_energy;
                // −p_i × log2(p_i) in millibits ≈ p_num/p_den × (bit_len(p_den)−bit_len(p_num)) × 1000
                let log2_p_den = u64::BITS - p_den.leading_zeros();
                let log2_p_num = u64::BITS - p_num.leading_zeros();
                let log2_ratio = log2_p_den.saturating_sub(log2_p_num) as u64;
                (p_num as u128 * log2_ratio as u128 * 1_000 / p_den as u128) as u64
            })
            .fold(0, u64::saturating_add)
    };

    // λ_eff = avg_λ − entropy_correction.
    // entropy_correction = entropy_mb × entropy_scale_mb / 1_000_000.
    let correction = (entropy_mb as u128 * params.entropy_scale_mb as u128 / 1_000_000) as u64;
    let lambda_eff = avg_lambda.saturating_sub(correction);

    let height_start = window.first().map(|b| b.height).unwrap_or(0);
    let height_end = window.last().map(|b| b.height).unwrap_or(0);

    Ok(EffectiveParams {
        step,
        height_start,
        height_end,
        lambda_eff,
        effective_accounts,
        energy_density,
        entropy_mb,
    })
}

/// Apply multiple successive RG steps to a block history, returning one
/// `EffectiveParams` per step.  The input `blocks` is consumed in
/// non-overlapping windows of `params.coarse_grain`.
///
/// Trailing blocks that don't fill a complete window are discarded.
pub fn rg_flow(blocks: &[BlockSummary], params: &RgFlowParams) -> Vec<EffectiveParams> {
    if params.coarse_grain == 0 {
        return vec![];
    }
    blocks
        .chunks_exact(params.coarse_grain)
        .enumerate()
        .filter_map(|(step, window)| rg_step(window, step, params).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::BlockSummary;

    fn block(height: u64, energy: u64, accounts: u64, lambda: u64) -> BlockSummary {
        BlockSummary {
            height,
            total_energy: energy,
            active_accounts: accounts,
            lambda_half_life: lambda,
        }
    }

    #[test]
    fn empty_window_errors() {
        let p = RgFlowParams {
            coarse_grain: 1,
            entropy_scale_mb: 0,
        };
        assert!(rg_step(&[], 0, &p).is_err());
    }

    #[test]
    fn length_mismatch_errors() {
        let p = RgFlowParams {
            coarse_grain: 3,
            entropy_scale_mb: 0,
        };
        let w = vec![block(0, 100, 10, 4096), block(1, 200, 20, 4096)];
        assert!(rg_step(&w, 0, &p).is_err());
    }

    #[test]
    fn uniform_window_preserves_lambda() {
        let p = RgFlowParams {
            coarse_grain: 4,
            entropy_scale_mb: 0,
        }; // no entropy correction
        let w: Vec<_> = (0..4).map(|i| block(i, 1_000, 10, 4096)).collect();
        let ep = rg_step(&w, 0, &p).unwrap();
        assert_eq!(ep.lambda_eff, 4096); // no correction when entropy_scale_mb=0
        assert_eq!(ep.energy_density, 1_000);
        assert_eq!(ep.effective_accounts, 10);
    }

    #[test]
    fn entropy_correction_decreases_lambda() {
        let p = RgFlowParams {
            coarse_grain: 2,
            entropy_scale_mb: 1_000_000,
        };
        // Two blocks with very different energies → high entropy → large correction.
        let w = vec![block(0, 1, 10, 4096), block(1, 1_000_000, 10, 4096)];
        let ep = rg_step(&w, 0, &p).unwrap();
        assert!(
            ep.lambda_eff < 4096,
            "entropy correction should reduce lambda_eff; got {}",
            ep.lambda_eff
        );
    }

    #[test]
    fn rg_flow_produces_n_div_coarse_steps() {
        let p = RgFlowParams {
            coarse_grain: 4,
            entropy_scale_mb: 0,
        };
        let blocks: Vec<_> = (0..12).map(|i| block(i, 1_000, 5, 4096)).collect();
        let flow = rg_flow(&blocks, &p);
        assert_eq!(flow.len(), 3); // 12 / 4 = 3
    }

    #[test]
    fn rg_flow_discards_trailing_blocks() {
        let p = RgFlowParams {
            coarse_grain: 4,
            entropy_scale_mb: 0,
        };
        let blocks: Vec<_> = (0..13).map(|i| block(i, 1_000, 5, 4096)).collect();
        let flow = rg_flow(&blocks, &p);
        assert_eq!(flow.len(), 3); // 13 → 3 complete windows
    }

    #[test]
    fn step_indices_are_sequential() {
        let p = RgFlowParams {
            coarse_grain: 2,
            entropy_scale_mb: 0,
        };
        let blocks: Vec<_> = (0..8).map(|i| block(i, 1_000, 5, 4096)).collect();
        let flow = rg_flow(&blocks, &p);
        for (i, ep) in flow.iter().enumerate() {
            assert_eq!(ep.step, i);
        }
    }
}
