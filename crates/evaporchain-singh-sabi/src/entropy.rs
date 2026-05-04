//! Visual-entropy state derivation.
//!
//! `PatinaState` is the canonical, validator-agreed-upon tuple
//! describing the token's *appearance* at a given epoch. The off-chain
//! shader consumes the tuple plus `token_id` (used as a deterministic
//! random seed) to render the actual cracks/foxing/desaturation —
//! validators don't have to agree on pixels, only on the entropy
//! tuple.
//!
//! Each entropy axis is normalised to `[0, 100]` (basis points / 100):
//!
//! - `cracks` — fraction of canvas overlaid with kintsugi-style cracks.
//!   0 = pristine, 100 = densely cracked.
//! - `desaturation` — colour drift toward greyscale. 0 = original
//!   palette, 100 = grayscale.
//! - `foxing` — paper-aging stains (the brown spots that develop on
//!   old paper). 0 = none, 100 = heavy foxing.
//! - `edge_fray` — how torn / frayed the canvas edges look. 0 =
//!   crisp, 100 = ragged.
//!
//! Each axis is a deterministic monotone-non-decreasing function of
//! the *aged fraction* `aged = (initial - score) / (initial - floor)`,
//! clipped to `[0, 1]`. Different axes have different rates so the
//! token's appearance changes texture over time, not just intensity:
//!
//! - `desaturation` rises fastest (linear in `aged`)
//! - `cracks` and `foxing` rise as `aged^2` (slow start, fast finish)
//! - `edge_fray` rises as `aged^3` (only really visible near the
//!   asymptote — torn-edge canvases are a "ruined-beautiful" signature)

use evaporchain_types::Energy;
use serde::{Deserialize, Serialize};

use crate::patina::PatinaParams;

/// Per-axis entropy in basis points / 100 (0..=100).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatinaState {
    pub cracks: u8,
    pub desaturation: u8,
    pub foxing: u8,
    pub edge_fray: u8,
    /// Echoed numeric score for convenience (the same value the shader
    /// uses to set overall brightness).
    pub score: Energy,
}

/// Derive `PatinaState` from `(score, params)`. Pure, deterministic,
/// validator-agreed.
pub fn derive_state(params: &PatinaParams, score: Energy) -> PatinaState {
    // aged_pct ∈ [0, 100]: how much of the decayable portion has aged.
    let decayable = params.initial - params.floor;
    let aged_pct: u32 = if decayable == 0 {
        0
    } else {
        let aged_amount = params.initial.saturating_sub(score);
        let pct = (aged_amount as u128 * 100) / (decayable as u128);
        pct.min(100) as u32
    };

    // Linear axis.
    let desaturation = aged_pct as u8;
    // Quadratic axes (aged_pct^2 / 100, clipped).
    let q = (aged_pct * aged_pct) / 100;
    let cracks = q.min(100) as u8;
    let foxing = q.min(100) as u8;
    // Cubic axis (aged_pct^3 / 10000, clipped).
    let cubic = (aged_pct * aged_pct * aged_pct) / 10_000;
    let edge_fray = cubic.min(100) as u8;

    PatinaState {
        cracks,
        desaturation,
        foxing,
        edge_fray,
        score,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> PatinaParams {
        // initial=1000, floor=150 (15%), half_life irrelevant here.
        PatinaParams::new(1000, 150, 100).unwrap()
    }

    #[test]
    fn at_mint_state_is_pristine() {
        let p = params();
        let s = derive_state(&p, p.initial);
        assert_eq!(s.cracks, 0);
        assert_eq!(s.desaturation, 0);
        assert_eq!(s.foxing, 0);
        assert_eq!(s.edge_fray, 0);
        assert_eq!(s.score, 1000);
    }

    #[test]
    fn at_floor_state_is_fully_aged() {
        let p = params();
        let s = derive_state(&p, p.floor);
        // aged_pct=100; desaturation=100 linear; cracks/foxing=100 quadratic;
        // edge_fray=100 cubic.
        assert_eq!(s.desaturation, 100);
        assert_eq!(s.cracks, 100);
        assert_eq!(s.foxing, 100);
        assert_eq!(s.edge_fray, 100);
        assert_eq!(s.score, p.floor);
    }

    #[test]
    fn below_floor_clamps_to_floor_state() {
        // Defensive: even if a caller hands in score < floor (shouldn't
        // happen via patina_score but the function is permissive),
        // entropy should clamp at fully-aged.
        let p = params();
        let s = derive_state(&p, 0);
        assert_eq!(s.desaturation, 100);
        assert_eq!(s.cracks, 100);
    }

    #[test]
    fn axes_have_different_curves_at_midpoint() {
        // At aged_pct=50, expected:
        //   desaturation = 50 (linear)
        //   cracks = 25 (quadratic)
        //   foxing = 25 (quadratic)
        //   edge_fray = 12 (cubic; 50^3 / 10000 = 125000 / 10000 = 12.5 → 12)
        let p = params();
        // score that gives aged_pct=50: aged = 0.5 * decayable = 425.
        // So score = initial - 425 = 575.
        let s = derive_state(&p, 575);
        assert_eq!(s.desaturation, 50);
        assert_eq!(s.cracks, 25);
        assert_eq!(s.foxing, 25);
        assert_eq!(s.edge_fray, 12);
    }

    #[test]
    fn above_initial_score_clamps_to_pristine() {
        // Caller passes a score above initial — defensive clamp.
        let p = params();
        let s = derive_state(&p, p.initial + 5_000);
        assert_eq!(s.cracks, 0);
        assert_eq!(s.desaturation, 0);
        assert_eq!(s.edge_fray, 0);
    }

    #[test]
    fn degenerate_zero_decayable_is_always_pristine() {
        let p = PatinaParams::new(500, 500, 50).unwrap();
        for score in [0, 100, 500, 1_000_000] {
            let s = derive_state(&p, score);
            assert_eq!(s.cracks, 0, "score {score}");
            assert_eq!(s.desaturation, 0, "score {score}");
        }
    }

    #[test]
    fn round_trip_serde() {
        let p = params();
        let s = derive_state(&p, 600);
        let j = serde_json::to_string(&s).unwrap();
        let back: PatinaState = serde_json::from_str(&j).unwrap();
        assert_eq!(s, back);
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// All entropy axes are monotone non-decreasing as `score`
        /// drops (i.e. as the token ages).
        #[test]
        fn entropy_monotone_in_aging(
            initial in 100u64..1_000_000,
            floor_frac in 0u64..100,
            score_a in 0u64..1_000_000,
            extra in 0u64..1_000_000,
        ) {
            let floor = (initial * floor_frac) / 100;
            let p = PatinaParams::new(initial, floor, 100).unwrap();
            // Pick score_a, and a "more-aged" score_b = score_a - extra.
            let score_a = score_a.min(initial);
            let score_b = score_a.saturating_sub(extra);
            let a = derive_state(&p, score_a);
            let b = derive_state(&p, score_b);
            prop_assert!(b.cracks >= a.cracks);
            prop_assert!(b.desaturation >= a.desaturation);
            prop_assert!(b.foxing >= a.foxing);
            prop_assert!(b.edge_fray >= a.edge_fray);
        }

        /// Axes are all in [0, 100].
        #[test]
        fn entropy_axes_in_unit_range(
            initial in 100u64..1_000_000,
            floor_frac in 0u64..100,
            score in 0u64..2_000_000,
        ) {
            let floor = (initial * floor_frac) / 100;
            let p = PatinaParams::new(initial, floor, 100).unwrap();
            let s = derive_state(&p, score);
            prop_assert!(s.cracks <= 100);
            prop_assert!(s.desaturation <= 100);
            prop_assert!(s.foxing <= 100);
            prop_assert!(s.edge_fray <= 100);
        }
    }
}
