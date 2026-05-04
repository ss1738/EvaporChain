//! Singh-Resonance coupling-curve → Sentinel parameter adapter.
//!
//! Two of `CouplingParams`'s fields are exactly the kind of
//! parameter Sentinel exists to govern continuously:
//!
//! - **`saturation`** — the attention quantity that maps to the
//!   mid-scale (1×) effective half-life. Below it, attention is
//!   subsidised toward decay; above, attention is subsidised toward
//!   immortality. Different communities will want different
//!   saturation set-points; Sentinel lets validators vote it within
//!   hard-coded bounds.
//!
//! - **`max_scale_bp`** — the asymptotic ceiling. *This is the
//!   anti-Black-Mirror knob*: governance can soften the ceiling if
//!   the community decides the chain has been too punishing, but the
//!   chain refuses to remove it entirely. Hard upper bound = 50× base
//!   half-life — beyond which the doctrine claim "slows toward
//!   immortality, but never pins to it" collapses.
//!
//! Both parameters are exposed; the rest of `CouplingParams`
//! (`min_scale_bp`, `mid_scale_bp`) are *structural* — moving them
//! changes the curve shape, not its set-point — and so are not
//! Sentinel-governed in V1.

use evaporchain_sentinel::{BoundedParameter, ParameterError};
use thiserror::Error;

/// Parameter id for the Resonance saturation knob.
pub const RESONANCE_SATURATION_PARAMETER_ID: u32 = 0x5E50_0000; // "RESO" hex'ish
/// Parameter id for the Resonance max-scale (anti-Black-Mirror) knob.
pub const RESONANCE_MAX_SCALE_PARAMETER_ID: u32 = 0x5E50_0001;

/// Saturation bounds. Below 100 the curve becomes degenerate (every
/// stray view saturates the wallet); above 100k it's so high that
/// virtually no token is ever in the loved range. The corridor keeps
/// the curve operationally meaningful.
pub const RESONANCE_SATURATION_MIN: u64 = 100;
pub const RESONANCE_SATURATION_MAX: u64 = 100_000;
pub const RESONANCE_SATURATION_DEFAULT: u64 = 1000;

/// Max-scale bounds, in basis points (×100). Doctrine default 800
/// (= 8× base half-life). Hard bounds:
/// - **`min = 200`** (2× base) — below this, "loved art slows" loses
///   meaning since the difference between min and max scale is too
///   small to feel.
/// - **`max = 5000`** (50× base) — above this, the doctrine
///   anti-Black-Mirror claim "never pins to it" is mathematically
///   honoured (asymptote stays asymptote) but functionally violated
///   (a token at 50× base half-life is *practically* immortal in
///   any meaningful timescale). The chain refuses to cross this line.
pub const RESONANCE_MAX_SCALE_MIN_BP: u64 = 200;
pub const RESONANCE_MAX_SCALE_MAX_BP: u64 = 5_000;
pub const RESONANCE_MAX_SCALE_DEFAULT_BP: u64 = 800;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ResonanceGovernanceError {
    #[error("parameter setup failed: {0}")]
    Parameter(#[from] ParameterError),
    #[error(
        "saturation {value} outside [{}, {}]",
        RESONANCE_SATURATION_MIN, RESONANCE_SATURATION_MAX
    )]
    SaturationOutOfBounds { value: u64 },
    #[error(
        "max_scale_bp {value} outside [{}, {}]",
        RESONANCE_MAX_SCALE_MIN_BP, RESONANCE_MAX_SCALE_MAX_BP
    )]
    MaxScaleOutOfBounds { value: u64 },
}

/// Build the `BoundedParameter` for the Resonance saturation knob.
pub fn resonance_saturation_parameter(
    initial: u64,
) -> Result<BoundedParameter, ResonanceGovernanceError> {
    if !(RESONANCE_SATURATION_MIN..=RESONANCE_SATURATION_MAX).contains(&initial) {
        return Err(ResonanceGovernanceError::SaturationOutOfBounds { value: initial });
    }
    Ok(BoundedParameter::new(
        RESONANCE_SATURATION_PARAMETER_ID,
        initial,
        RESONANCE_SATURATION_MIN,
        RESONANCE_SATURATION_MAX,
    )?)
}

/// Build the `BoundedParameter` for the Resonance max-scale
/// (anti-Black-Mirror ceiling) knob.
pub fn resonance_max_scale_parameter(
    initial_bp: u64,
) -> Result<BoundedParameter, ResonanceGovernanceError> {
    if !(RESONANCE_MAX_SCALE_MIN_BP..=RESONANCE_MAX_SCALE_MAX_BP).contains(&initial_bp) {
        return Err(ResonanceGovernanceError::MaxScaleOutOfBounds { value: initial_bp });
    }
    Ok(BoundedParameter::new(
        RESONANCE_MAX_SCALE_PARAMETER_ID,
        initial_bp,
        RESONANCE_MAX_SCALE_MIN_BP,
        RESONANCE_MAX_SCALE_MAX_BP,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doctrine_default_saturation_is_within_bounds() {
        let p = resonance_saturation_parameter(RESONANCE_SATURATION_DEFAULT).unwrap();
        assert_eq!(p.current, 1000);
        assert_eq!(p.min, 100);
        assert_eq!(p.max, 100_000);
        assert_eq!(p.id, RESONANCE_SATURATION_PARAMETER_ID);
    }

    #[test]
    fn doctrine_default_max_scale_is_within_bounds() {
        let p = resonance_max_scale_parameter(RESONANCE_MAX_SCALE_DEFAULT_BP).unwrap();
        assert_eq!(p.current, 800);
        assert_eq!(p.min, 200);
        assert_eq!(p.max, 5_000);
        assert_eq!(p.id, RESONANCE_MAX_SCALE_PARAMETER_ID);
    }

    #[test]
    fn saturation_too_low_rejected() {
        let err = resonance_saturation_parameter(50).unwrap_err();
        assert!(matches!(err, ResonanceGovernanceError::SaturationOutOfBounds { .. }));
    }

    #[test]
    fn saturation_too_high_rejected() {
        let err = resonance_saturation_parameter(1_000_000).unwrap_err();
        assert!(matches!(err, ResonanceGovernanceError::SaturationOutOfBounds { .. }));
    }

    #[test]
    fn max_scale_below_anti_blackmirror_floor_rejected() {
        // Below 200 bp = below 2× base → "loved art slows" loses force.
        let err = resonance_max_scale_parameter(100).unwrap_err();
        assert!(matches!(err, ResonanceGovernanceError::MaxScaleOutOfBounds { .. }));
    }

    #[test]
    fn max_scale_above_practical_immortality_rejected() {
        // Above 5000 bp = above 50× base → asymptote stays an
        // asymptote, but functionally tokens become immortal. The
        // chain refuses to cross this line.
        let err = resonance_max_scale_parameter(10_000).unwrap_err();
        assert!(matches!(err, ResonanceGovernanceError::MaxScaleOutOfBounds { .. }));
    }

    #[test]
    fn parameter_ids_are_distinct_and_stable() {
        // Anti-collision regression marker.
        assert_ne!(
            RESONANCE_SATURATION_PARAMETER_ID,
            RESONANCE_MAX_SCALE_PARAMETER_ID
        );
        // Stable across releases.
        assert_eq!(RESONANCE_SATURATION_PARAMETER_ID, 0x5E50_0000);
        assert_eq!(RESONANCE_MAX_SCALE_PARAMETER_ID, 0x5E50_0001);
    }

    #[test]
    fn the_anti_black_mirror_corridor_is_enforced() {
        // Doctrine: validators can soften the ceiling, but cannot
        // remove it. Outside [200, 5000] bp, the chain refuses; the
        // ceiling is *governable but bounded*.
        for bp in [0, 50, 199, 5_001, 9_999, 50_000] {
            assert!(
                resonance_max_scale_parameter(bp).is_err(),
                "bp={bp} should be out of corridor"
            );
        }
        for bp in [200, 400, 800, 2_000, 5_000] {
            assert!(
                resonance_max_scale_parameter(bp).is_ok(),
                "bp={bp} should be in corridor"
            );
        }
    }
}
