//! `Beacon { e4, e6, delta }` triple + the modular-identity check.
//!
//! Per the doctrine, beacon outputs satisfy `E_4³ − E_6² = 1728 · Δ`
//! at every τ. The truncated computation breaks this identity past
//! the truncation depth — but at q = 0 the relation reduces to
//! `1 − 1 = 0 = 1728 · 0`, which the substrate verifies exactly.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::evaluate::{evaluate_delta, evaluate_e4, evaluate_e6};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Beacon {
    pub tau: u64,
    pub e4: i128,
    pub e6: i128,
    pub delta: i128,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BeaconError {
    #[error(
        "modular identity violated: E_4³ ({e4_cubed}) - E_6² ({e6_squared}) ≠ 1728·Δ ({delta_scaled}); residual={residual}; tolerance={tolerance}"
    )]
    IdentityFailed {
        e4_cubed: i128,
        e6_squared: i128,
        delta_scaled: i128,
        residual: i128,
        tolerance: i128,
    },
}

/// Compute the per-epoch beacon at `tau`. `tau` is the VRF-supplied
/// q-value (interpreted as a small unsigned integer — the substrate
/// stand-in for the analytic `q = e^(2πiτ)`).
pub fn compute_beacon(tau: u64) -> Beacon {
    Beacon {
        tau,
        e4: evaluate_e4(tau),
        e6: evaluate_e6(tau),
        delta: evaluate_delta(tau),
    }
}

/// Verify `E_4³ − E_6² = 1728·Δ` within an absolute `tolerance`.
/// At q = 0 the relation is exact (residual = 0). For larger q the
/// truncated polynomial breaks the identity by a residual that grows
/// with q — caller picks the tolerance appropriate for their
/// truncation/q regime.
pub fn verify_modular_identity(beacon: &Beacon, tolerance: i128) -> Result<(), BeaconError> {
    let e4_cubed = beacon
        .e4
        .saturating_mul(beacon.e4)
        .saturating_mul(beacon.e4);
    let e6_squared = beacon.e6.saturating_mul(beacon.e6);
    let delta_scaled = 1728i128.saturating_mul(beacon.delta);
    let lhs = e4_cubed.saturating_sub(e6_squared);
    let residual = lhs.saturating_sub(delta_scaled).abs();
    if residual <= tolerance {
        Ok(())
    } else {
        Err(BeaconError::IdentityFailed {
            e4_cubed,
            e6_squared,
            delta_scaled,
            residual,
            tolerance,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beacon_at_tau_zero_is_canonical() {
        let b = compute_beacon(0);
        assert_eq!(b.tau, 0);
        assert_eq!(b.e4, 1);
        assert_eq!(b.e6, 1);
        assert_eq!(b.delta, 0);
    }

    #[test]
    fn modular_identity_exact_at_tau_zero() {
        // E_4(0)^3 - E_6(0)^2 = 1 - 1 = 0 = 1728 * Δ(0) = 0. Exact.
        let b = compute_beacon(0);
        verify_modular_identity(&b, 0).expect("exact at τ=0");
    }

    #[test]
    fn beacon_at_tau_one_has_known_values() {
        let b = compute_beacon(1);
        // E_4(1) = sum of E4_COEFFS = 199_920.
        let e4_sum: i128 = 1 + 240 + 2160 + 6720 + 17520 + 30240 + 60480 + 82560;
        assert_eq!(b.e4, e4_sum);
        // Δ(1) = 65275.
        assert_eq!(b.delta, 65275);
    }

    #[test]
    fn modular_identity_breaks_past_truncation_at_large_tau() {
        // At τ=2 the truncation residual is significant; tolerance=0
        // should fail.
        let b = compute_beacon(2);
        assert!(verify_modular_identity(&b, 0).is_err());
    }
}
