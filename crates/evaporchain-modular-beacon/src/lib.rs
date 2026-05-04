//! Modular-form beacon — Eisenstein E_4, E_6, and the modular
//! discriminant Δ at a VRF-supplied per-epoch τ.
//!
//! Per `research/INVENTION_STACK.md` §A1.4:
//!
//! > **Modular-Form Beacon** | Zagier; Eisenstein E_k(τ), modular
//! > discriminant Δ(τ) | Per-epoch beacon = (E_4, E_6, Δ) at τ_epoch
//! > from VRF. Outputs satisfy known modular equations — aperiodic,
//! > hard to fake without solving the modular equation, cheap to
//! > verify. q-expansion in `q = e^(2πiτ)` reframes naturally as
//! > `e^(−λt)`.
//!
//! ## Substrate scope
//!
//! Real modular-form evaluation requires q ∈ unit disk in ℂ; for
//! chain-deterministic integer arithmetic we use truncated q-expansions
//! over `u128` with `q` interpreted as a small unsigned integer (so
//! `q^k` is plain integer exponentiation, modulo overflow saturation).
//!
//! The output triple satisfies the *modular relation*
//!
//! ```text
//!   E_4(τ)^3 − E_6(τ)^2  =  1728 · Δ(τ)
//! ```
//!
//! at every τ. The check `verify_modular_identity` validates this
//! relation holds for the truncated computation up to a tolerance
//! that accounts for the truncation depth.
//!
//! ## Module map
//!
//! - [`coeffs`] — precomputed q-expansion coefficient tables for
//!   E_4 / E_6 / Δ.
//! - [`evaluate`] — `evaluate_e4(q)`, `evaluate_e6(q)`, `evaluate_delta(q)`
//!   truncated-series.
//! - [`beacon`] — `compute_beacon(tau)` returns the `Beacon` triple +
//!   `verify_modular_identity(beacon)`.

pub mod beacon;
pub mod coeffs;
pub mod evaluate;

pub use beacon::{compute_beacon, verify_modular_identity, Beacon, BeaconError};
pub use coeffs::{DELTA_COEFFS, E4_COEFFS, E6_COEFFS, TRUNCATION_DEPTH};
pub use evaluate::{evaluate_delta, evaluate_e4, evaluate_e6};

#[cfg(test)]
mod press_claim_tests {
    use super::*;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "Modular-Form Beacon emits the (E_4, E_6, Δ)
    /// triple at a per-epoch τ. The triple satisfies the modular
    /// relation E_4³ − E_6² = 1728·Δ EXACTLY at τ=0 (the substrate's
    /// canonical zero point). Different τs yield different triples
    /// (no aliasing), and `compute_beacon` is pure (same τ → same
    /// triple)."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Modular identity exact at τ=0.
        let b0 = compute_beacon(0);
        assert_eq!(b0.tau, 0);
        verify_modular_identity(&b0, 0).expect("modular identity exact at τ=0");

        // Determinism: same input → same triple.
        let b0_again = compute_beacon(0);
        assert_eq!(b0, b0_again);

        // Distinct τs produce distinct beacons (no aliasing on small τ).
        let b1 = compute_beacon(1);
        let b2 = compute_beacon(2);
        assert_ne!(b0, b1);
        assert_ne!(b1, b2);

        // Tight tolerance fails past truncation regime — verifier
        // gets a typed error, not silent acceptance.
        let b_far = compute_beacon(2);
        assert!(matches!(
            verify_modular_identity(&b_far, 0),
            Err(BeaconError::IdentityFailed { .. })
        ));
    }
}
