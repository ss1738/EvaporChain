//! Decay-state predicates that gate vault payout.
//!
//! V1 ships two predicates. Both evaluate purely from a
//! `PredicateContext` carrying consensus-time + the vault's current
//! energy reading. No oracle, no system clock — every input is on-chain.
//!
//! Adding new predicate variants is non-breaking as long as the enum
//! stays `#[non_exhaustive]` and `evaluate` is updated.

use evaporchain_types::{energy_at_epoch, Energy, Epoch, HalfLife};
use serde::{Deserialize, Serialize};

/// A predicate that, when true at a given epoch, releases the vault.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Predicate {
    /// Release at consensus epoch ≥ `release_epoch`.
    EpochReached { release_epoch: Epoch },
    /// Release when the lot's energy (computed via the standard
    /// exponential half-life rule) has decayed strictly below
    /// `threshold`. The `initial_energy`, `half_life`, and
    /// `created_at_epoch` fields are baked in at vault construction so
    /// later evaluation is deterministic.
    EnergyDecaysBelow {
        initial_energy: Energy,
        half_life: HalfLife,
        created_at_epoch: Epoch,
        threshold: Energy,
    },
}

/// Snapshot of evaluation inputs at a given epoch. Validators agree on
/// these inputs because they're derived from the chain head.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PredicateContext {
    pub epoch_now: Epoch,
}

/// True iff the predicate is satisfied at `ctx.epoch_now`.
pub fn evaluate(p: &Predicate, ctx: PredicateContext) -> bool {
    match *p {
        Predicate::EpochReached { release_epoch } => ctx.epoch_now >= release_epoch,
        Predicate::EnergyDecaysBelow {
            initial_energy,
            half_life,
            created_at_epoch,
            threshold,
        } => {
            let elapsed = ctx.epoch_now.saturating_sub(created_at_epoch);
            let now_energy = energy_at_epoch(initial_energy, half_life, elapsed);
            now_energy < threshold
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_reached_false_before_release() {
        let p = Predicate::EpochReached { release_epoch: 100 };
        assert!(!evaluate(&p, PredicateContext { epoch_now: 99 }));
    }

    #[test]
    fn epoch_reached_true_at_and_after_release() {
        let p = Predicate::EpochReached { release_epoch: 100 };
        assert!(evaluate(&p, PredicateContext { epoch_now: 100 }));
        assert!(evaluate(&p, PredicateContext { epoch_now: 9999 }));
    }

    #[test]
    fn energy_decays_below_false_at_creation() {
        // initial_energy=1000, threshold=500, just-created → 1000 ≥ 500
        let p = Predicate::EnergyDecaysBelow {
            initial_energy: 1000,
            half_life: 50,
            created_at_epoch: 0,
            threshold: 500,
        };
        assert!(!evaluate(&p, PredicateContext { epoch_now: 0 }));
    }

    #[test]
    fn energy_decays_below_eventually_true() {
        // After many half-lives, energy → 0; predicate must trip.
        let p = Predicate::EnergyDecaysBelow {
            initial_energy: 1000,
            half_life: 10,
            created_at_epoch: 0,
            threshold: 1, // strictly below 1 ⇒ 0
        };
        assert!(evaluate(&p, PredicateContext { epoch_now: 10_000 }));
    }

    #[test]
    fn energy_decays_below_threshold_zero_only_when_fully_decayed() {
        // threshold=0 means "strictly less than 0" — unsatisfiable for
        // u64. Defensive: even after infinite epochs, predicate stays false.
        let p = Predicate::EnergyDecaysBelow {
            initial_energy: 1,
            half_life: 1,
            created_at_epoch: 0,
            threshold: 0,
        };
        assert!(!evaluate(&p, PredicateContext { epoch_now: u64::MAX }));
    }

    #[test]
    fn round_trip_serde() {
        let p = Predicate::EnergyDecaysBelow {
            initial_energy: 5000,
            half_life: 100,
            created_at_epoch: 42,
            threshold: 100,
        };
        let s = serde_json::to_string(&p).unwrap();
        let back: Predicate = serde_json::from_str(&s).unwrap();
        assert_eq!(p, back);
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// EpochReached is monotone: if true at t, true at all t' > t.
        #[test]
        fn epoch_reached_monotone(release in 0u64..1_000_000, t in 0u64..2_000_000, extra in 0u64..1_000_000) {
            let p = Predicate::EpochReached { release_epoch: release };
            let v_t = evaluate(&p, PredicateContext { epoch_now: t });
            let v_t2 = evaluate(&p, PredicateContext { epoch_now: t.saturating_add(extra) });
            if v_t {
                prop_assert!(v_t2);
            }
        }

        /// EnergyDecaysBelow is monotone: once below, stays below
        /// (since energy_at_epoch is non-increasing in elapsed).
        #[test]
        fn energy_below_monotone(
            initial in 1u64..1_000_000,
            half_life in 1u64..10_000,
            t in 0u64..1_000_000,
            extra in 0u64..1_000_000,
            threshold in 1u64..1_000_000,
        ) {
            let p = Predicate::EnergyDecaysBelow {
                initial_energy: initial,
                half_life,
                created_at_epoch: 0,
                threshold,
            };
            let v_t = evaluate(&p, PredicateContext { epoch_now: t });
            let v_t2 = evaluate(&p, PredicateContext { epoch_now: t.saturating_add(extra) });
            if v_t {
                prop_assert!(v_t2);
            }
        }
    }
}
