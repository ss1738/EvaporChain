//! Decay-state predicates that gate vault payout.
//!
//! **EvaporScript-first reconciliation (2026-05-16).** The canonical
//! source of truth for SFSV business logic is the EvaporScript contract
//! `contracts/evaporscript/future_self_vault.es` (EvaporChain invariant
//! #2). Its `EnergyDecaysBelow` predicate reads the *contract's own
//! built-in `energy`* — the live value the evaporation engine decays and
//! `on_refresh` resets — and compares it to `threshold`:
//!
//! ```text
//!   if energy < self.threshold { satisfied = 1 }
//! ```
//!
//! This module therefore does **not** re-derive decay from frozen
//! `(initial_energy, half_life, created_at)` params. Re-deriving decay
//! outside the engine (a) violated invariant #1 (energy must route
//! through the engine, not be recomputed) and (b) was refresh-blind, so
//! a boosted vault released differently from the deployed `.es`. The
//! predicate is now a *pure comparison* over an engine-supplied,
//! refresh-inclusive `contract_energy` reading carried in
//! `PredicateContext`. Decay and refresh are the engine's job; the
//! predicate only asks "is the live energy below threshold yet?".
//!
//! V1 ships two predicates; both evaluate purely from on-chain inputs
//! (consensus epoch + the vault contract's live energy). No oracle, no
//! system clock, no recomputed formula.

use evaporchain_types::{Energy, Epoch};
use serde::{Deserialize, Serialize};

/// A predicate that, when true at a given epoch, releases the vault.
///
/// Mirrors the `.es` `predicate_type` field: `EpochReached` = type 0,
/// `EnergyDecaysBelow` = type 1. Only the release parameter is stored —
/// exactly as the `.es` keeps `release_epoch` / `threshold`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Predicate {
    /// Release at consensus epoch ≥ `release_epoch`. (`.es` type 0.)
    EpochReached { release_epoch: Epoch },
    /// Release when the vault contract's *live* energy — the value the
    /// evaporation engine decays and `on_refresh` resets — is strictly
    /// below `threshold`. (`.es` type 1; reads the built-in `energy`.)
    EnergyDecaysBelow { threshold: Energy },
}

/// Snapshot of evaluation inputs at a given epoch. Validators agree on
/// these because both are derived from the chain head: `epoch_now` from
/// consensus, `contract_energy` from the vault instance's engine-tracked
/// energy field (post-decay, post-refresh — the same value the `.es`
/// sees as `energy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PredicateContext {
    pub epoch_now: Epoch,
    /// The vault contract's current engine-tracked energy. The caller
    /// (payout/execution layer) supplies the live value; this module
    /// never recomputes decay itself (invariant #1).
    pub contract_energy: Energy,
}

/// True iff the predicate is satisfied under `ctx`. Pure; no side
/// effects. Bit-for-bit equivalent to the `.es` `try_payout` /
/// `predicate_satisfied` inline logic:
///   type 0 → `epoch >= release_epoch`
///   type 1 → `energy < threshold`
pub fn evaluate(p: &Predicate, ctx: PredicateContext) -> bool {
    match *p {
        Predicate::EpochReached { release_epoch } => ctx.epoch_now >= release_epoch,
        Predicate::EnergyDecaysBelow { threshold } => ctx.contract_energy < threshold,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(epoch_now: Epoch, contract_energy: Energy) -> PredicateContext {
        PredicateContext {
            epoch_now,
            contract_energy,
        }
    }

    #[test]
    fn epoch_reached_false_before_release() {
        let p = Predicate::EpochReached { release_epoch: 100 };
        assert!(!evaluate(&p, ctx(99, 0)));
    }

    #[test]
    fn epoch_reached_true_at_and_after_release() {
        let p = Predicate::EpochReached { release_epoch: 100 };
        assert!(evaluate(&p, ctx(100, 999)));
        assert!(evaluate(&p, ctx(9999, 999)));
    }

    #[test]
    fn energy_below_false_while_energy_at_or_above_threshold() {
        // Mirrors `.es`: `energy < threshold`. 1000 ≥ 500 ⇒ not satisfied.
        let p = Predicate::EnergyDecaysBelow { threshold: 500 };
        assert!(!evaluate(&p, ctx(0, 1000)));
        assert!(!evaluate(&p, ctx(99_999, 500))); // exactly 500 is NOT < 500
    }

    #[test]
    fn energy_below_true_once_engine_energy_drops() {
        // The engine decayed the vault's live energy below threshold.
        let p = Predicate::EnergyDecaysBelow { threshold: 500 };
        assert!(evaluate(&p, ctx(1234, 499)));
        assert!(evaluate(&p, ctx(1234, 0)));
    }

    #[test]
    fn energy_below_is_refresh_aware() {
        // Doctrine fix: a boosted (`on_refresh`'d) vault whose live
        // energy is back above threshold must NOT release — the old
        // frozen-formula predicate could not express this.
        let p = Predicate::EnergyDecaysBelow { threshold: 500 };
        // decayed below → would release
        assert!(evaluate(&p, ctx(2000, 400)));
        // ...but a refresh pushed live energy back to 900 at a later
        // epoch ⇒ must be false again (refresh-aware).
        assert!(!evaluate(&p, ctx(2100, 900)));
    }

    #[test]
    fn energy_below_threshold_zero_is_unsatisfiable() {
        // `energy < 0` is impossible for u64 — matches `.es` semantics
        // (threshold 0 ⇒ never trips, even fully evaporated).
        let p = Predicate::EnergyDecaysBelow { threshold: 0 };
        assert!(!evaluate(&p, ctx(u64::MAX, 0)));
    }

    #[test]
    fn round_trip_serde() {
        for p in [
            Predicate::EpochReached { release_epoch: 42 },
            Predicate::EnergyDecaysBelow { threshold: 100 },
        ] {
            let s = serde_json::to_string(&p).unwrap();
            let back: Predicate = serde_json::from_str(&s).unwrap();
            assert_eq!(p, back);
        }
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// EpochReached is monotone in epoch: true at t ⇒ true at t' ≥ t
        /// (energy is irrelevant for this variant).
        #[test]
        fn epoch_reached_monotone(
            release in 0u64..1_000_000,
            t in 0u64..2_000_000,
            extra in 0u64..1_000_000,
            e in 0u64..1_000_000,
        ) {
            let p = Predicate::EpochReached { release_epoch: release };
            let v_t = evaluate(&p, PredicateContext { epoch_now: t, contract_energy: e });
            let v_t2 = evaluate(&p, PredicateContext {
                epoch_now: t.saturating_add(extra), contract_energy: e,
            });
            if v_t { prop_assert!(v_t2); }
        }

        /// EnergyDecaysBelow is monotone in energy (NOT in epoch — it is
        /// deliberately refresh-aware, so it can flip back to false if a
        /// boost raises live energy). Lower energy ⇒ "more satisfied".
        #[test]
        fn energy_below_monotone_in_energy(
            threshold in 1u64..1_000_000,
            hi in 0u64..1_000_000,
            drop in 0u64..1_000_000,
            t in 0u64..1_000_000,
        ) {
            let lo = hi.saturating_sub(drop);
            let p = Predicate::EnergyDecaysBelow { threshold };
            let v_hi = evaluate(&p, PredicateContext { epoch_now: t, contract_energy: hi });
            let v_lo = evaluate(&p, PredicateContext { epoch_now: t, contract_energy: lo });
            // If satisfied at higher energy, must stay satisfied at lower.
            if v_hi { prop_assert!(v_lo); }
        }
    }
}
