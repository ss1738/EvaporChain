//! Patina-floor → Sentinel parameter adapter.
//!
//! Singh-Sabi's "ruined-beautiful" floor (default 15% of initial) is
//! exactly the kind of parameter Sentinel exists to govern: a
//! creator-tunable knob with a sensible-default range and a
//! cultural-claim attached. Wrap it as a `BoundedParameter` with
//! hard bounds the chain refuses to cross even under sustained
//! validator vote.
//!
//! ## Why bounds
//!
//! Per A2.5 + the sentinel doctrine:
//!
//! > Each governable parameter has hard-coded `(min, max)` bounds
//! > set at genesis. Within those bounds, validators vote
//! > continuously on the parameter's value.
//!
//! For the patina floor:
//! - **`min = 5%` (500 bp)** — below this the asymptote becomes
//!   indistinguishable from a Mayfly (genuine death) and Singh-Sabi's
//!   "ruined-beautiful" framing collapses.
//! - **`max = 50%` (5_000 bp)** — above this the token barely
//!   decays and stops being a Singh-Sabi token in any meaningful
//!   sense.
//!
//! These bounds are themselves changeable only via LLSA proof per
//! the doctrine ("any bounds change requires a Coq-checked LLSA
//! proof that the new bounds preserve chain invariants"). Out of
//! scope here — this crate only exposes the parameter object.

use evaporchain_sentinel::{BoundedParameter, ParameterError};
use thiserror::Error;

/// Stable parameter ID for the patina-floor knob. Must not collide
/// with other governable parameters; chosen at the high end of the
/// 32-bit space to keep room for substrate parameters in the low.
pub const PATINA_FLOOR_PARAMETER_ID: u32 = 0x53AB_1F00; // "SAB1F00" — Sabi-Floor

/// Doctrine bounds (basis points / 100, i.e. 100 bp = 1%).
///
/// `min = 5%` keeps the floor distinct from a Mayfly's zero-floor.
/// `max = 50%` keeps the token meaningfully a Singh-Sabi (i.e.
/// genuinely decaying, not just lightly aging).
pub const PATINA_FLOOR_MIN_BP: u64 = 5; // 5%
pub const PATINA_FLOOR_MAX_BP: u64 = 50; // 50%
/// Doctrine default — the wabi-sabi-anchored 15%.
pub const PATINA_FLOOR_DEFAULT_BP: u64 = 15;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PatinaGovernanceError {
    #[error("parameter setup failed: {0}")]
    Parameter(#[from] ParameterError),
    #[error(
        "requested initial value {bp} outside [{}, {}]",
        PATINA_FLOOR_MIN_BP,
        PATINA_FLOOR_MAX_BP
    )]
    OutOfBounds { bp: u64 },
}

/// Build a `BoundedParameter` for the patina floor, starting at
/// `initial_bp` (validated against the hard bounds). Returns the
/// chain-set parameter handle the Sentinel controller can then
/// adjust toward the decay-weighted-average vote.
pub fn patina_floor_parameter(initial_bp: u64) -> Result<BoundedParameter, PatinaGovernanceError> {
    if !(PATINA_FLOOR_MIN_BP..=PATINA_FLOOR_MAX_BP).contains(&initial_bp) {
        return Err(PatinaGovernanceError::OutOfBounds { bp: initial_bp });
    }
    Ok(BoundedParameter::new(
        PATINA_FLOOR_PARAMETER_ID,
        initial_bp,
        PATINA_FLOOR_MIN_BP,
        PATINA_FLOOR_MAX_BP,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doctrine_default_is_within_bounds() {
        let p = patina_floor_parameter(PATINA_FLOOR_DEFAULT_BP).unwrap();
        assert_eq!(p.current, 15);
        assert_eq!(p.min, 5);
        assert_eq!(p.max, 50);
        assert_eq!(p.id, PATINA_FLOOR_PARAMETER_ID);
    }

    #[test]
    fn rejects_floor_too_low() {
        let err = patina_floor_parameter(2).unwrap_err();
        assert!(matches!(err, PatinaGovernanceError::OutOfBounds { .. }));
    }

    #[test]
    fn rejects_floor_too_high() {
        let err = patina_floor_parameter(80).unwrap_err();
        assert!(matches!(err, PatinaGovernanceError::OutOfBounds { .. }));
    }

    #[test]
    fn min_and_max_are_inclusive_endpoints() {
        // Boundary check — both endpoints valid.
        let lo = patina_floor_parameter(PATINA_FLOOR_MIN_BP).unwrap();
        assert_eq!(lo.current, PATINA_FLOOR_MIN_BP);
        let hi = patina_floor_parameter(PATINA_FLOOR_MAX_BP).unwrap();
        assert_eq!(hi.current, PATINA_FLOOR_MAX_BP);
    }

    #[test]
    fn bounds_define_the_singh_sabi_corridor() {
        // Doctrine commentary made operational: outside the corridor,
        // Singh-Sabi loses its meaning. Below 5% the asymptote
        // collapses toward Mayfly; above 50% the decay is too gentle
        // to be Singh-Sabi at all. The parameter's hard bounds
        // enforce the corridor.
        for bp in [0, 1, 4, 51, 99, 100] {
            assert!(
                patina_floor_parameter(bp).is_err(),
                "bp={bp} should be out of corridor"
            );
        }
        for bp in [5, 10, 15, 25, 50] {
            assert!(
                patina_floor_parameter(bp).is_ok(),
                "bp={bp} should be in corridor"
            );
        }
    }

    #[test]
    fn parameter_id_is_stable_constant() {
        // Anti-collision regression marker: changing the parameter
        // ID would silently invalidate every existing on-chain vote
        // for the patina floor.
        assert_eq!(PATINA_FLOOR_PARAMETER_ID, 0x53AB_1F00);
    }
}
