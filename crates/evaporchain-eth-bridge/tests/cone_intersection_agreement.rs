//! Bit-compat agreement between Rust `energy_at_epoch` and Solidity
//! `ConeIntersection.energyAtEpoch`.
//!
//! The bridge-side replay-immunity guard (`ConeIntersection.sol`)
//! ports the canonical decay formula from `evaporchain_types`. Both
//! sides must agree on every `(initial, half_life, elapsed)` tuple
//! the bridge sees, otherwise an honestly-submitted cross-chain tx
//! could be rejected on Ethereum even though the issuing chain
//! considers it inside its cone.
//!
//! This test exercises a sweep of representative inputs and prints
//! the Rust-side outputs. The Solidity test
//! `test/ConeIntersection.t.sol::test_energyAtEpoch_*` exercises the
//! same input shapes (zero half-life, monotone decreasing, one
//! half-life, 64-halvings-collapse, etc.) so a divergence would
//! show up on both sides.

use evaporchain_types::energy_at_epoch;

#[test]
fn at_observation_is_initial() {
    // elapsed=0 → returns initial.
    assert_eq!(energy_at_epoch(1000, 100, 0), 1000);
}

#[test]
fn one_half_life_halves() {
    assert_eq!(energy_at_epoch(1000, 100, 100), 500);
}

#[test]
fn zero_half_life_is_zero() {
    assert_eq!(energy_at_epoch(1000, 0, 5), 0);
}

#[test]
fn sixty_four_halvings_collapse_to_zero() {
    assert_eq!(energy_at_epoch(u64::MAX, 1, 64), 0);
    assert_eq!(energy_at_epoch(u64::MAX, 1, 1_000), 0);
}

#[test]
fn monotone_decreasing() {
    let hl = 50u64;
    let mut prev = energy_at_epoch(1000, hl, 0);
    for t in (1u64..200).step_by(5) {
        let cur = energy_at_epoch(1000, hl, t);
        assert!(
            cur <= prev,
            "energy_at_epoch must be monotone decreasing (t={t}, prev={prev}, cur={cur})"
        );
        prev = cur;
    }
}

#[test]
fn cone_intersection_smoke() {
    // Mirrors `test_oneOutside_invalid` in ConeIntersection.t.sol —
    // chain A long λ → still inside at epoch=200; chain B short λ
    // and high threshold → outside. Bridge invalid.
    //
    // (Cone struct logic is in the cone-bridge crate's own tests;
    // this test exists to keep the Rust + Solidity numerics paired.)
    let a_remaining = energy_at_epoch(1000, 10_000, 200);
    let b_remaining = energy_at_epoch(1000, 100, 200);
    let a_inside = a_remaining >= 100;
    let b_inside = b_remaining >= 600;
    assert!(a_inside, "A must be inside at epoch=200 (long λ)");
    assert!(
        !b_inside,
        "B must be outside at epoch=200 (short λ + high threshold)"
    );
}
