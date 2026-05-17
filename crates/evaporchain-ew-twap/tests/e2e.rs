//! End-to-end integration tests for evaporchain-ew-twap.
//!
//! Non-trivial fixture: "Honest-dominated spot market".
//!
//!   Two honest oracles attest at high energy; one attacker
//!   submits a wildly off-market price at 1000x lower energy.
//!   Window = 100 epochs.
//!
//!   Honest oracle A: epoch 10, price=1_000_000, energy=100_000
//!   Honest oracle B: epoch 20, price=3_000_000, energy=100_000
//!   Attacker X:      epoch 30, price=10_000_000, energy=100
//!
//!   EW-TWAP exact arithmetic:
//!     sum_pe = 1_000_000*100_000 + 3_000_000*100_000 + 10_000_000*100
//!            = 100_000_000_000 + 300_000_000_000 + 1_000_000_000
//!            = 401_000_000_000
//!     sum_e  = 100_000 + 100_000 + 100 = 200_100
//!     EW-TWAP = 401_000_000_000 / 200_100 = 2_003_997 (integer division)
//!       (check: 200_100 * 2_003_997 = 401_199_769_700 -- wait let me recompute)
//!
//!   200_100 * 2_003_997:
//!     200_100 * 2_000_000 = 400_200_000_000
//!     200_100 * 3_997     = 200_100 * 4_000 - 200_100 * 3
//!                         = 800_400_000 - 600_300 = 799_799_700
//!     total               = 400_200_000_000 + 799_799_700 = 400_999_799_700
//!     remainder           = 401_000_000_000 - 400_999_799_700 = 200_300
//!     200_300 / 200_100 = 1 remainder 200
//!     EW-TWAP = 2_003_998
//!
//!   Verification: 200_100 * 2_003_998 = 400_999_799_700 + 200_100 = 400_999_999_800
//!     remainder = 401_000_000_000 - 400_999_999_800 = 200
//!     Yes: EW-TWAP = 2_003_998 (200 / 200_100 < 1, so integer division stops here).
//!
//!   Standard TWAP (equal weights): (1_000_000+3_000_000+10_000_000)/3 = 4_666_666
//!   EW-TWAP (2_003_998) vs standard TWAP (4_666_666):
//!     attacker's off-market price barely moves EW-TWAP from honest mean 2_000_000.
//!     standard TWAP gives attacker 2.3x the distortion.
//!
//!   Window eviction: observations older than `window_epochs` are pruned.
//!   If the chain advances 200 epochs without new data, read_at(210)
//!   prunes obs at epoch 10 (cutoff=210-100=110 > 10). Only obs 20 and 30 survive.
//!   After further advance to epoch 200+, all obs pruned -> NoObservations.
//!
//! Doctrine claim (INVENTION_STACK.md §4.3 / EW-TWAP):
//! "EW-TWAP weights observations by energy-at-observation, not elapsed
//!  time. A short, high-energy attestation outweighs a longer, low-energy
//!  one. Manipulation cost scales with the chain's single-lambda decay
//!  rate -- sustaining a manipulated price requires re-attesting it with
//!  fresh energy every epoch. The oracle is pure-integer fixed-point and
//!  validator-deterministic; an empty window is an explicit error, never
//!  a silent zero."
//!
//! Adversarial fixture: NonMonotoneEpoch, NoObservations, ZeroTotalEnergy, Overflow.
//!
//! INVENTION_STACK §4.3: EW-TWAP (Tier-3 App-Layer).

use evaporchain_ew_twap::{EwTwapOracle, OracleError, Observation};

// -- Helpers ------------------------------------------------------------------

fn obs(epoch: u64, price: u64, energy: u64) -> Observation {
    Observation::new(epoch, price, energy)
}

// -- Non-trivial fixture ------------------------------------------------------

#[test]
fn fixture_honest_dominated_spot_market() {
    // Two honest oracles at high energy vs. one attacker at low energy.
    let mut oracle = EwTwapOracle::new(100);

    oracle.observe(obs(10, 1_000_000, 100_000)).unwrap();
    oracle.observe(obs(20, 3_000_000, 100_000)).unwrap();
    oracle.observe(obs(30, 10_000_000, 100)).unwrap(); // attacker

    let twap = oracle.read().unwrap();
    // sum_pe = 100B + 300B + 1B = 401_000_000_000
    // sum_e  = 200_100
    // twap   = 401_000_000_000 / 200_100 = 2_003_998
    assert_eq!(twap, 2_003_998, "EW-TWAP exact arithmetic");

    // Honest mean (ignoring attacker) = (1M + 3M) / 2 = 2_000_000.
    // EW-TWAP is only 3_998 above honest mean -- attacker barely moves it.
    assert!(
        twap < 2_100_000,
        "attacker's price (10M) should barely move EW-TWAP; got {twap}"
    );

    // Standard TWAP would be (1M + 3M + 10M) / 3 = 4_666_666.
    // EW-TWAP is 2.3x closer to honest mean than standard TWAP.
    let standard_twap = (1_000_000u64 + 3_000_000 + 10_000_000) / 3;
    assert_eq!(standard_twap, 4_666_666);
    let ew_deviation = twap.abs_diff(2_000_000);
    let std_deviation = standard_twap.abs_diff(2_000_000);
    assert!(
        ew_deviation < std_deviation,
        "EW-TWAP ({twap}) must be closer to honest mean than standard TWAP ({standard_twap})"
    );
}

#[test]
fn fixture_window_eviction_removes_stale_obs() {
    // Window = 50 epochs. At epoch 110, obs at epoch 10 falls out
    // (cutoff = 110-50 = 60 > 10).
    let mut oracle = EwTwapOracle::new(50);

    oracle.observe(obs(10, 1_000_000, 1_000)).unwrap(); // will be evicted at epoch 110+
    oracle.observe(obs(60, 2_000_000, 1_000)).unwrap(); // still in window at 110
    oracle.observe(obs(80, 3_000_000, 1_000)).unwrap(); // in window at 110

    // At epoch 110: cutoff=60, obs(10) is evicted (10 < 60).
    // Only obs(60) and obs(80) remain.
    let twap = oracle.read_at(110).unwrap();
    // sum_pe = 2_000_000*1_000 + 3_000_000*1_000 = 5_000_000_000
    // sum_e  = 2_000
    // twap   = 5_000_000_000 / 2_000 = 2_500_000
    assert_eq!(twap, 2_500_000, "window eviction removes stale epoch 10 obs");

    // At epoch 10_000: all observations evicted. NoObservations.
    let err = oracle.read_at(10_000).unwrap_err();
    assert_eq!(err, OracleError::NoObservations, "fully stale window");
}

#[test]
fn fixture_manipulation_attack_vs_honest_baseline() {
    // Adversary submits 20 low-energy off-market attestations.
    // One honest high-energy attestation dominates.
    let mut oracle = EwTwapOracle::new(10_000);

    // Attacker: 20 epochs, price = 9_000_000, energy = 50 each.
    for i in 1..=20u64 {
        oracle.observe(obs(i, 9_000_000, 50)).unwrap();
    }
    // Honest: epoch 21, price = 1_000_000, energy = 100_000.
    oracle.observe(obs(21, 1_000_000, 100_000)).unwrap();

    // Attacker total weight: 20 * 50 = 1_000.
    // Honest total weight:   100_000.
    // Attacker weight fraction: 1_000 / 101_000 < 1%.
    let twap = oracle.read().unwrap();
    assert!(
        twap < 1_100_000,
        "honest observation (100k energy) dominates 20 attacker obs (1k total); got {twap}"
    );
}

#[test]
fn fixture_equal_energy_gives_arithmetic_mean() {
    // When all energies are equal, EW-TWAP collapses to arithmetic mean.
    let mut oracle = EwTwapOracle::new(0); // no-window

    oracle.observe(obs(1, 1_000_000, 500)).unwrap();
    oracle.observe(obs(2, 2_000_000, 500)).unwrap();
    oracle.observe(obs(3, 3_000_000, 500)).unwrap();

    let twap = oracle.read().unwrap();
    // sum_pe = (1M + 2M + 3M) * 500 = 3_000_000_000
    // sum_e  = 1_500
    // twap   = 3_000_000_000 / 1_500 = 2_000_000
    assert_eq!(twap, 2_000_000, "equal-energy EW-TWAP = arithmetic mean");
}

// -- Doctrine tests -----------------------------------------------------------

#[test]
fn doctrine_validator_determinism() {
    // Same observations submitted in two oracles -> identical reading.
    let mut o1 = EwTwapOracle::new(100);
    let mut o2 = EwTwapOracle::new(100);

    for (ep, price, energy) in [(5, 1_000_000, 500), (10, 2_000_000, 800), (15, 3_000_000, 200)] {
        o1.observe(obs(ep, price, energy)).unwrap();
        o2.observe(obs(ep, price, energy)).unwrap();
    }
    assert_eq!(o1.read().unwrap(), o2.read().unwrap());
}

#[test]
fn doctrine_ew_twap_in_observed_price_range() {
    // EW-TWAP is a convex combination; must lie within [min_price, max_price].
    let mut oracle = EwTwapOracle::new(0);

    oracle.observe(obs(1, 500_000,   200)).unwrap();
    oracle.observe(obs(2, 800_000,   500)).unwrap();
    oracle.observe(obs(3, 1_200_000, 300)).unwrap();

    let twap = oracle.read().unwrap();
    assert!(twap >= 500_000,   "EW-TWAP must be >= min observed price");
    assert!(twap <= 1_200_000, "EW-TWAP must be <= max observed price");
}

#[test]
fn doctrine_empty_window_is_error_not_zero() {
    // Core doctrine: NoObservations, not 0.
    let oracle = EwTwapOracle::new(100);
    assert_eq!(oracle.read().unwrap_err(), OracleError::NoObservations);
    assert_eq!(oracle.read_at(50).unwrap_err(), OracleError::NoObservations);
}

#[test]
fn doctrine_higher_energy_dominates_lower() {
    // The high-energy observation should dominate the result.
    let mut oracle = EwTwapOracle::new(0);

    oracle.observe(obs(1, 1_000_000, 10)).unwrap();      // low energy, low price
    oracle.observe(obs(2, 2_000_000, 100_000)).unwrap(); // high energy, high price

    let twap = oracle.read().unwrap();
    // The high-energy obs has 10_000x the weight -- result should be very close to 2M.
    assert!(twap > 1_999_000, "high-energy obs must dominate; got {twap}");
}

// -- Adversarial fixture ------------------------------------------------------

#[test]
fn adversarial_non_monotone_epoch_rejected() {
    let mut oracle = EwTwapOracle::new(100);
    oracle.observe(obs(10, 1_000_000, 1_000)).unwrap();

    // Same epoch: rejected.
    assert_eq!(
        oracle.observe(obs(10, 2_000_000, 1_000)).unwrap_err(),
        OracleError::NonMonotoneEpoch { incoming: 10, last: 10 }
    );

    // Earlier epoch: rejected.
    let err = oracle.observe(obs(5, 2_000_000, 1_000)).unwrap_err();
    assert!(matches!(err, OracleError::NonMonotoneEpoch { incoming: 5, .. }));
}

#[test]
fn adversarial_zero_total_energy_rejected() {
    let mut oracle = EwTwapOracle::new(0);
    oracle.observe(obs(1, 1_000_000, 0)).unwrap();
    oracle.observe(obs(2, 2_000_000, 0)).unwrap();
    assert_eq!(oracle.read().unwrap_err(), OracleError::ZeroTotalEnergy);
    assert_eq!(oracle.read_at(5).unwrap_err(), OracleError::ZeroTotalEnergy);
}

#[test]
fn adversarial_u64_max_overflow_detected() {
    // Two u64::MAX observations overflow u128 accumulator on sum.
    let mut oracle = EwTwapOracle::new(0);
    oracle.observe(obs(1, u64::MAX, u64::MAX)).unwrap();
    // Single: (2^64-1)^2 < 2^128, reads OK.
    assert_eq!(oracle.read().unwrap(), u64::MAX);

    // Adding a second: accumulated sum overflows u128.
    oracle.observe(obs(2, u64::MAX, u64::MAX)).unwrap();
    assert_eq!(oracle.read().unwrap_err(), OracleError::Overflow);
}

// -- Cross-cuts ---------------------------------------------------------------

#[test]
fn serde_round_trip_preserves_oracle() {
    let mut oracle = EwTwapOracle::new(100);
    oracle.observe(obs(5, 1_000_000, 500)).unwrap();
    oracle.observe(obs(10, 2_000_000, 700)).unwrap();

    let json = serde_json::to_string(&oracle).unwrap();
    let back: EwTwapOracle = serde_json::from_str(&json).unwrap();
    assert_eq!(oracle.read().unwrap(), back.read().unwrap());
}

#[test]
fn prune_at_standalone_is_idempotent() {
    let mut oracle = EwTwapOracle::new(10);
    oracle.observe(obs(5, 1_000_000, 1_000)).unwrap();
    oracle.observe(obs(10, 2_000_000, 1_000)).unwrap();
    assert_eq!(oracle.observation_count(), 2);

    // Prune: cutoff = 20 - 10 = 10. Obs at epoch 5 (< 10) dropped.
    oracle.prune_at(20);
    assert_eq!(oracle.observation_count(), 1);

    // Idempotent.
    oracle.prune_at(20);
    assert_eq!(oracle.observation_count(), 1);
}
