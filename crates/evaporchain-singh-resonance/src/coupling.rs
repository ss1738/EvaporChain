//! Engagement → effective half-life coupling.
//!
//! The doctrine claim: λ is *inversely coupled* to engagement. Loved
//! art slows; ignored art accelerates. Concretely, V1 ships a
//! piecewise scaling on the token's *base* half-life, parameterised
//! by the current attention quantity:
//!
//! ```text
//!     effective_half_life(base, attention, params)
//!         = base * scale(attention / params.saturation)
//! ```
//!
//! Where `scale` is monotone non-decreasing in normalized attention,
//! capped at `params.max_scale_bp / 100` so a single viral moment
//! can't pin a token to immortality. Unwitnessed tokens
//! (`attention == 0`) get the *minimum* scale (≥ 1 to avoid
//! 0-half-life).
//!
//! Doctrine defaults (basis points / 100):
//! - `min_scale_bp = 50` — ignored art decays at 0.5× the base half-life
//!   (i.e. dies twice as fast)
//! - `mid_scale_bp = 100` — engagement at saturation gives the base rate
//! - `max_scale_bp = 800` — sustained-attention ceiling: 8× the base
//!   half-life (loved art *slows toward* immortality, not pins to it)
//! - `saturation = 1000` — attention units that produce mid scale; the
//!   chain operator picks this to match expected engagement volumes
//!
//! These are deliberate choices, not arbitrary. Min < 1 means
//! ignored art is *worse* than the base case; Max ≫ 1 means loved art
//! gets a real reward. The asymptote at max enforces "slows toward,"
//! not "becomes immortal" — a Black-Mirror guard.

use evaporchain_types::HalfLife;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CouplingError {
    #[error("base_half_life must be > 0")]
    ZeroBaseHalfLife,
    #[error("saturation must be > 0")]
    ZeroSaturation,
    #[error("min_scale_bp must be > 0 (else half-life can hit 0)")]
    ZeroMinScale,
    #[error("max_scale_bp must be ≥ mid_scale_bp")]
    InvertedMaxMid,
    #[error("mid_scale_bp must be ≥ min_scale_bp")]
    InvertedMidMin,
}

/// Coupling parameters. Doctrine defaults via `default_attention_curve`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CouplingParams {
    /// Scale (×100, basis-point) at zero attention. Tokens that nobody
    /// witnesses use this; default 50 = 0.5× base half-life (decay 2× faster).
    pub min_scale_bp: u32,
    /// Scale at `attention == saturation`. Default 100 = base rate.
    pub mid_scale_bp: u32,
    /// Scale ceiling; sustained massive engagement asymptotes here.
    /// Default 800 = 8× base half-life (loved art slows toward
    /// immortality, but never crosses it — Black-Mirror guard).
    pub max_scale_bp: u32,
    /// Attention unit that maps to mid_scale. Caller picks based on
    /// expected engagement volume.
    pub saturation: u64,
}

impl CouplingParams {
    pub fn new(
        min_scale_bp: u32,
        mid_scale_bp: u32,
        max_scale_bp: u32,
        saturation: u64,
    ) -> Result<Self, CouplingError> {
        if saturation == 0 {
            return Err(CouplingError::ZeroSaturation);
        }
        if min_scale_bp == 0 {
            return Err(CouplingError::ZeroMinScale);
        }
        if mid_scale_bp < min_scale_bp {
            return Err(CouplingError::InvertedMidMin);
        }
        if max_scale_bp < mid_scale_bp {
            return Err(CouplingError::InvertedMaxMid);
        }
        Ok(Self {
            min_scale_bp,
            mid_scale_bp,
            max_scale_bp,
            saturation,
        })
    }

    /// Doctrine defaults: 0.5× / 1.0× / 8.0× scale, saturation = 1000.
    pub fn default_attention_curve(saturation: u64) -> Result<Self, CouplingError> {
        Self::new(50, 100, 800, saturation)
    }
}

/// Effective half-life given the token's `base_half_life`, current
/// `attention`, and coupling params. Closed-form, integer-only.
///
/// Curve: piecewise linear between (0, min) → (saturation, mid) →
/// (∞, max). Above saturation, we interpolate linearly toward max,
/// using `attention - saturation` measured in units of saturation;
/// after `~7 × saturation` we are essentially at max.
pub fn effective_half_life(
    base_half_life: HalfLife,
    attention: u64,
    params: &CouplingParams,
) -> Result<HalfLife, CouplingError> {
    if base_half_life == 0 {
        return Err(CouplingError::ZeroBaseHalfLife);
    }
    let scale_bp = piecewise_scale(attention, params);
    // half_life_eff = base * scale_bp / 100, with u128 intermediate.
    let scaled = (base_half_life as u128) * (scale_bp as u128) / 100;
    Ok(scaled.min(HalfLife::MAX as u128) as HalfLife)
}

/// Compute the scale in basis points for a given attention. Pure
/// function; deterministic; integer-only.
fn piecewise_scale(attention: u64, p: &CouplingParams) -> u32 {
    if attention == 0 {
        return p.min_scale_bp;
    }
    if attention <= p.saturation {
        // Linear: min → mid as attention goes 0 → saturation.
        let span = (p.mid_scale_bp - p.min_scale_bp) as u128;
        let bp = p.min_scale_bp as u128 + (span * attention as u128) / p.saturation as u128;
        return bp.min(u32::MAX as u128) as u32;
    }
    // Above saturation: asymptote toward max with diminishing returns.
    // Use the formula `mid + (max - mid) * (1 - 1/(1 + n))` where
    // `n = attention / saturation`. Integer realisation:
    //   over_n = attention / saturation (integer)
    //   over_r = attention % saturation
    //   approach = (max - mid) * over_n / (over_n + 1)   for the integer part
    // This gives exact mid at attention = saturation (over_n = 1 ⇒
    // approach = (max-mid)/2 ⇒ wait that's not what we want at
    // saturation. We want the boundary to evaluate to mid). Reframe:
    //   above = attention - saturation
    //   approach = (max - mid) * above / (above + saturation)
    // At above=0 (boundary): approach = 0 → scale = mid. ✓
    // At above → ∞: approach → max - mid → scale → max. ✓
    let above = (attention - p.saturation) as u128;
    let span = (p.max_scale_bp - p.mid_scale_bp) as u128;
    let approach = span * above / (above + p.saturation as u128);
    let bp = p.mid_scale_bp as u128 + approach;
    bp.min(u32::MAX as u128) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doctrine_params() -> CouplingParams {
        CouplingParams::default_attention_curve(1000).unwrap()
    }

    #[test]
    fn rejects_zero_base_half_life() {
        let p = doctrine_params();
        assert_eq!(
            effective_half_life(0, 100, &p).unwrap_err(),
            CouplingError::ZeroBaseHalfLife
        );
    }

    #[test]
    fn rejects_inverted_min_max() {
        let err = CouplingParams::new(800, 100, 50, 1000).unwrap_err();
        assert!(matches!(err, CouplingError::InvertedMidMin));
    }

    #[test]
    fn rejects_zero_min_scale() {
        let err = CouplingParams::new(0, 100, 800, 1000).unwrap_err();
        assert_eq!(err, CouplingError::ZeroMinScale);
    }

    #[test]
    fn ignored_token_decays_faster_than_base() {
        // Doctrine: ignored art accelerates toward zero.
        // attention=0 ⇒ scale = min_scale = 0.5× ⇒ effective HL is
        // half the base ⇒ decay is 2× faster.
        let p = doctrine_params();
        let h = effective_half_life(1000, 0, &p).unwrap();
        assert_eq!(h, 500); // 0.5× base
    }

    #[test]
    fn at_saturation_token_decays_at_base_rate() {
        let p = doctrine_params();
        let h = effective_half_life(1000, 1000, &p).unwrap();
        assert_eq!(h, 1000); // 1.0× base = base
    }

    #[test]
    fn loved_token_slows_toward_immortality_but_never_reaches_max() {
        // Doctrine: "slows toward immortality" — must approach the
        // ceiling but never cross it. The Black-Mirror guard.
        let p = doctrine_params();
        // Massive attention.
        let h = effective_half_life(1000, 1_000_000, &p).unwrap();
        // 8× base = 8000.
        let max_possible = 8000;
        assert!(h < max_possible, "must not reach max: {h}");
        assert!(h > 1000, "must be much bigger than mid: {h}");
        // Very close to but below max.
        assert!(
            h > 7000,
            "should be approaching max for very high attention: {h}"
        );
    }

    #[test]
    fn scale_is_monotone_in_attention() {
        let p = doctrine_params();
        let h_zero = effective_half_life(1000, 0, &p).unwrap();
        let h_low = effective_half_life(1000, 100, &p).unwrap();
        let h_mid = effective_half_life(1000, 1000, &p).unwrap();
        let h_high = effective_half_life(1000, 10_000, &p).unwrap();
        let h_huge = effective_half_life(1000, 1_000_000, &p).unwrap();
        assert!(h_zero <= h_low);
        assert!(h_low <= h_mid);
        assert!(h_mid <= h_high);
        assert!(h_high <= h_huge);
    }

    #[test]
    fn at_zero_attention_uses_min_scale_exactly() {
        let p = CouplingParams::new(60, 100, 500, 1000).unwrap();
        let h = effective_half_life(1000, 0, &p).unwrap();
        assert_eq!(h, 600); // 0.6× base
    }

    #[test]
    fn round_trip_serde() {
        let p = doctrine_params();
        let s = serde_json::to_string(&p).unwrap();
        let back: CouplingParams = serde_json::from_str(&s).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn the_anti_black_mirror_test() {
        // Doctrine guardrail: "slows TOWARD immortality, but never
        // pins to it." Even adversarial sustained attention can't push
        // the effective half-life past max_scale × base.
        let p = doctrine_params();
        let max_hl_possible = 1000 * 800 / 100; // 8000
        for adversarial_attention in [10_000_u64, 1_000_000, u64::MAX / 2] {
            let h = effective_half_life(1000, adversarial_attention, &p).unwrap();
            assert!(
                h < max_hl_possible,
                "attention={adversarial_attention} produced {h} ≥ ceiling {max_hl_possible}"
            );
        }
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Effective half-life is monotone non-decreasing in attention.
        #[test]
        fn monotone_in_attention(
            base in 1u64..1_000_000,
            attention_a in 0u64..2_000_000,
            extra in 0u64..2_000_000,
        ) {
            let p = CouplingParams::default_attention_curve(1000).unwrap();
            let h_a = effective_half_life(base, attention_a, &p).unwrap();
            let h_b =
                effective_half_life(base, attention_a.saturating_add(extra), &p).unwrap();
            prop_assert!(h_b >= h_a, "{h_a} → {h_b}");
        }

        /// Effective half-life is in [min_scale × base, max_scale × base).
        #[test]
        fn bounded_by_min_and_max_scale(
            base in 1u64..100_000,
            attention in 0u64..2_000_000,
        ) {
            let p = CouplingParams::default_attention_curve(1000).unwrap();
            let h = effective_half_life(base, attention, &p).unwrap();
            let lower = base * p.min_scale_bp as u64 / 100;
            let upper = base * p.max_scale_bp as u64 / 100;
            prop_assert!(h >= lower);
            prop_assert!(h < upper, "h={h} must be < upper={upper} (asymptote, not equal)");
        }
    }
}
