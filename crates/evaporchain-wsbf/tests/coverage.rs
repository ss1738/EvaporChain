//! Coverage tests for Wilson-Singh Block Flow (WSBF) — RG-flow on
//! chain history (Tier 2, `INVENTION_STACK.md §4.2`). Each RG step
//! integrates out the highest-energy short-distance degrees of
//! freedom, rescaling λ → λ_eff.
//!
//! Existing in-module tests cover the happy path (uniform window,
//! entropy correction, flow-count, step indices) and two error
//! variants. This file adds:
//!
//!   - `rg_flow` degenerate inputs (zero coarse_grain, empty blocks)
//!   - `rg_step` field-output invariants (height range, density,
//!     accounts, zero-total entropy path)
//!   - `rg_step` saturating_add safety on near-u64::MAX inputs
//!   - Skewed vs uniform Shannon entropy (skewed has LOWER H)
//!   - Successive non-overlapping windows advance correctly
//!   - Serde round-trip for BlockSummary + EffectiveParams + RgFlowParams
//!   - `RgFlowError` Display rendering

use evaporchain_wsbf::flow::{rg_flow, rg_step, RgFlowError};
use evaporchain_wsbf::params::{BlockSummary, EffectiveParams, RgFlowParams};

fn block(height: u64, energy: u64, accounts: u64, lambda: u64) -> BlockSummary {
    BlockSummary {
        height,
        total_energy: energy,
        active_accounts: accounts,
        lambda_half_life: lambda,
    }
}

// =================================================================
// rg_flow degenerate inputs
// =================================================================

#[test]
fn rg_flow_with_zero_coarse_grain_returns_empty() {
    let p = RgFlowParams { coarse_grain: 0, entropy_scale_mb: 0 };
    let blocks: Vec<_> = (0..10).map(|i| block(i, 1_000, 5, 4096)).collect();
    let flow = rg_flow(&blocks, &p);
    assert!(flow.is_empty(), "zero coarse_grain must short-circuit to empty");
}

#[test]
fn rg_flow_on_empty_blocks_returns_empty() {
    let p = RgFlowParams::default();
    let flow = rg_flow(&[], &p);
    assert!(flow.is_empty());
}

// =================================================================
// rg_step field invariants
// =================================================================

#[test]
fn rg_step_height_range_spans_window() {
    let p = RgFlowParams { coarse_grain: 4, entropy_scale_mb: 0 };
    let w: Vec<_> = (10..14).map(|i| block(i, 1_000, 5, 4096)).collect();
    let ep = rg_step(&w, 7, &p).unwrap();
    assert_eq!(ep.height_start, 10);
    assert_eq!(ep.height_end, 13);
    assert_eq!(ep.step, 7, "step index propagates verbatim");
}

#[test]
fn rg_step_zero_total_energy_yields_zero_entropy() {
    let p = RgFlowParams { coarse_grain: 3, entropy_scale_mb: 1_000_000 };
    let w = vec![block(0, 0, 0, 1_000), block(1, 0, 0, 1_000), block(2, 0, 0, 1_000)];
    let ep = rg_step(&w, 0, &p).unwrap();
    assert_eq!(ep.entropy_mb, 0, "zero total_energy short-circuits entropy to 0");
    assert_eq!(ep.lambda_eff, 1_000, "with no entropy, no correction → λ_eff = avg_λ");
    assert_eq!(ep.energy_density, 0);
    assert_eq!(ep.effective_accounts, 0);
}

#[test]
fn rg_step_energy_density_is_total_over_n() {
    let p = RgFlowParams { coarse_grain: 4, entropy_scale_mb: 0 };
    let w = vec![
        block(0, 100, 1, 4096),
        block(1, 200, 1, 4096),
        block(2, 300, 1, 4096),
        block(3, 400, 1, 4096),
    ];
    let ep = rg_step(&w, 0, &p).unwrap();
    assert_eq!(ep.energy_density, (100 + 200 + 300 + 400) / 4);
}

#[test]
fn rg_step_effective_accounts_is_total_over_n() {
    let p = RgFlowParams { coarse_grain: 3, entropy_scale_mb: 0 };
    let w = vec![
        block(0, 1_000, 5, 4096),
        block(1, 1_000, 10, 4096),
        block(2, 1_000, 21, 4096),
    ];
    let ep = rg_step(&w, 0, &p).unwrap();
    assert_eq!(ep.effective_accounts, (5 + 10 + 21) / 3);
}

#[test]
fn rg_step_total_energy_saturating_add_safety() {
    // Two near-u64::MAX values must not panic on the energy sum.
    // (The fold uses saturating_add, so the result clamps at u64::MAX.)
    let p = RgFlowParams { coarse_grain: 2, entropy_scale_mb: 0 };
    let w = vec![
        block(0, u64::MAX, 1, 1_000),
        block(1, u64::MAX, 1, 1_000),
    ];
    let ep = rg_step(&w, 0, &p).expect("must not panic on saturation");
    // energy_density = saturating-clamped sum / 2 = u64::MAX / 2
    assert_eq!(ep.energy_density, u64::MAX / 2);
}

#[test]
fn rg_step_single_block_window_succeeds() {
    let p = RgFlowParams { coarse_grain: 1, entropy_scale_mb: 0 };
    let w = vec![block(42, 9_999, 7, 4096)];
    let ep = rg_step(&w, 5, &p).unwrap();
    assert_eq!(ep.step, 5);
    assert_eq!(ep.height_start, 42);
    assert_eq!(ep.height_end, 42);
    assert_eq!(ep.energy_density, 9_999);
    assert_eq!(ep.effective_accounts, 7);
}

// =================================================================
// Shannon entropy: skewed vs uniform
// =================================================================

#[test]
fn rg_step_skewed_distribution_has_lower_entropy_than_uniform() {
    let p = RgFlowParams { coarse_grain: 2, entropy_scale_mb: 0 };
    let uniform = vec![
        block(0, 1_000_000, 1, 4096),
        block(1, 1_000_000, 1, 4096),
    ];
    let skewed = vec![
        block(0, 1, 1, 4096),
        block(1, 1_000_000, 1, 4096),
    ];
    let ep_uniform = rg_step(&uniform, 0, &p).unwrap();
    let ep_skewed = rg_step(&skewed, 0, &p).unwrap();
    assert!(
        ep_skewed.entropy_mb < ep_uniform.entropy_mb,
        "skewed distribution must yield LOWER Shannon entropy than uniform; \
         got skewed={}, uniform={}",
        ep_skewed.entropy_mb,
        ep_uniform.entropy_mb,
    );
}

// =================================================================
// rg_flow window stepping
// =================================================================

#[test]
fn rg_flow_advances_height_window_correctly() {
    let p = RgFlowParams { coarse_grain: 3, entropy_scale_mb: 0 };
    let blocks: Vec<_> = (0..9).map(|i| block(i, 1_000, 5, 4096)).collect();
    let flow = rg_flow(&blocks, &p);
    assert_eq!(flow.len(), 3);
    // Window 0: blocks 0..2 → height_start=0, end=2
    // Window 1: blocks 3..5 → height_start=3, end=5
    // Window 2: blocks 6..8 → height_start=6, end=8
    assert_eq!(flow[0].height_start, 0);
    assert_eq!(flow[0].height_end, 2);
    assert_eq!(flow[1].height_start, 3);
    assert_eq!(flow[1].height_end, 5);
    assert_eq!(flow[2].height_start, 6);
    assert_eq!(flow[2].height_end, 8);
}

// =================================================================
// Serde round-trips
// =================================================================

#[test]
fn block_summary_serde_round_trips() {
    let b = block(42, 9_999, 7, 4096);
    let json = serde_json::to_string(&b).expect("serialize");
    let back: BlockSummary = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.height, b.height);
    assert_eq!(back.total_energy, b.total_energy);
    assert_eq!(back.active_accounts, b.active_accounts);
    assert_eq!(back.lambda_half_life, b.lambda_half_life);
}

#[test]
fn effective_params_serde_round_trips() {
    let p = RgFlowParams { coarse_grain: 4, entropy_scale_mb: 0 };
    let w: Vec<_> = (10..14).map(|i| block(i, 1_000, 5, 4096)).collect();
    let ep = rg_step(&w, 3, &p).unwrap();
    let json = serde_json::to_string(&ep).expect("serialize");
    let back: EffectiveParams = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.step, ep.step);
    assert_eq!(back.height_start, ep.height_start);
    assert_eq!(back.height_end, ep.height_end);
    assert_eq!(back.lambda_eff, ep.lambda_eff);
    assert_eq!(back.effective_accounts, ep.effective_accounts);
    assert_eq!(back.energy_density, ep.energy_density);
    assert_eq!(back.entropy_mb, ep.entropy_mb);
}

#[test]
fn rg_flow_params_serde_round_trips() {
    let p = RgFlowParams { coarse_grain: 256, entropy_scale_mb: 4_096 };
    let json = serde_json::to_string(&p).expect("serialize");
    let back: RgFlowParams = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.coarse_grain, p.coarse_grain);
    assert_eq!(back.entropy_scale_mb, p.entropy_scale_mb);
}

// =================================================================
// RgFlowError Display
// =================================================================

#[test]
fn rg_flow_error_displays_both_variants() {
    let empty = RgFlowError::EmptyWindow.to_string();
    let mismatch = RgFlowError::WindowLengthMismatch(2, 5).to_string();
    assert!(empty.contains("empty"), "got: {empty}");
    assert!(
        mismatch.contains("2") && mismatch.contains("5"),
        "got: {mismatch}"
    );
}
