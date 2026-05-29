//! Decay-reputation substrate primitive.
//!
//! A signed reputation score built from two accumulators — `merit` and
//! `demerit` — that each decay through `evaporchain_types::
//! energy_at_epoch`. Net reputation is `merit - demerit`. Because both
//! good and bad standing fade at the same half-life, the score behaves
//! like an exponentially-weighted moving accumulator:
//!
//! - A **recent** event outweighs an equal-magnitude **stale** one.
//! - A subject can **live down a fault** by staying active — an old
//!   demerit decays toward irrelevance.
//! - Standing is never permanent: merit must keep being earned.
//!
//! This is distinct from the rest of the decay suite:
//! `decay-credential` is a single attestation with a validity floor,
//! `decay-quorum` aggregates member weights into a vote, and this is a
//! per-subject *signed* score with separate positive/negative decay.
//!
//! Two accumulators (rather than one signed value decayed in place) so
//! the canonical `energy_at_epoch` curve — which is defined on
//! non-negative energy — applies cleanly to each side; the sign lives
//! only in the `net` subtraction.
//!
//! ## Three structural decisions enforced as tests
//!
//! 1. **Both merit and demerit decay via `energy_at_epoch`.** Neither
//!    good nor bad reputation is permanent.
//!
//! 2. **Net is signed (`i128`).** Demerit can drive net negative;
//!    earning merit can flip it back positive.
//!
//! 3. **Recency dominates.** A fresh event of size `x` outweighs a
//!    stale event of the same size, because the stale one has decayed.
//!
//! ## Module map
//!
//! - [`reputation`] — [`Reputation`] dual-accumulator cell + [`RepError`].
//! - [`ledger`] — [`ReputationLedger`]: per-subject keyed scores +
//!   leaderboard + prune.

pub mod ledger;
pub mod reputation;

pub use ledger::ReputationLedger;
pub use reputation::{RepError, Reputation};

#[cfg(test)]
mod press_claim_tests {
    use super::*;

    /// Doctrine claim asserted as a structural test.
    ///
    /// Press claim: "Reputation on EvaporChain fades both ways. Good
    /// standing must be kept earned and a fault can be lived down — and
    /// a recent action counts for more than an equal stale one. A
    /// subject driven negative by a penalty can recover by earning."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let a = [0xAAu8; 32];
        let b = [0xBBu8; 32];
        let c = [0xCCu8; 32];
        let d = [0xDDu8; 32];

        let mut ledger = ReputationLedger::new(10).unwrap(); // half-life 10

        // a earns 1000, then takes a 500 fault — net 500 at t=0.
        ledger.record_merit(a, 1000, 0).unwrap();
        ledger.record_demerit(a, 500, 0).unwrap();
        assert_eq!(ledger.net(&a, 0), 500);
        // Both sides fade equally over one half-life → net halves.
        assert_eq!(ledger.net(&a, 10), 250);

        // Recency dominates: c earned 100 at t=0, b earns 100 at t=20.
        ledger.record_merit(c, 100, 0).unwrap();
        ledger.record_merit(b, 100, 20).unwrap();
        assert!(ledger.net(&b, 20) > ledger.net(&c, 20)); // 100 > 25

        // d is driven negative by a penalty, then recovers by earning.
        ledger.record_demerit(d, 1000, 0).unwrap();
        assert!(ledger.net(&d, 0) < 0);
        ledger.record_merit(d, 2000, 0).unwrap();
        assert!(ledger.net(&d, 0) > 0);
    }
}
