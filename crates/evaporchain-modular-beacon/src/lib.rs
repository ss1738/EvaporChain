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
