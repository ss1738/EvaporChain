//! The conservation invariant.
//!
//! Per INVENTION_STACK.md §1.2:
//!
//! > Total energy budget across {accounts + stake + refresh pool +
//! > slashed pool} decreases monotonically only via the global decay
//! > term λ — never by destruction in any other transition.
//!
//! Operationally, the chain transitions through three kinds of step:
//!
//!   1. **Redirect** (no epoch advance) — `EnergyAccumulator.total()` is
//!      preserved exactly. Asserted by `ConservationCheck::redirect`.
//!   2. **Decay** (epoch advance, no redirects) — `total()` may drop, but
//!      only by an amount consistent with `energy_at_epoch` applied at
//!      the chain λ to the previous total. Asserted by
//!      `ConservationCheck::decay_step`.
//!   3. **Composite block step** (both happen in one block) — accepted
//!      iff it can be expressed as a sequence of (1) and (2) steps. The
//!      coarse audit `ConservationCheck::block_step` collapses both into
//!      a single bound: `total_after ≤ total_before` AND any drop ≤ the
//!      maximum decay over `epochs_elapsed` at the chain λ.
//!
//! This is the operational form of the Coq theorem owed in
//! INVENTION_STACK.md §8 ("Conservation invariant proof — formal
//! Lyapunov-style proof that total energy is monotone-decreasing"). The
//! Rust check is what the runtime asserts; a future Coq mechanization
//! reduces to `energy_at_epoch_monotone` (already proven in
//! `research/coq/EnergyDecayMonotonicity.v`).

use thiserror::Error;

use crate::compartment::EnergyAccumulator;
use crate::lambda::ChainLambda;
use evaporchain_types::{energy_at_epoch, Energy};

#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum ConservationViolation {
    #[error("redirect step changed total: before={before}, after={after}")]
    RedirectChangedTotal { before: Energy, after: Energy },

    #[error("decay step total increased: before={before}, after={after}")]
    DecayIncreasedTotal { before: Energy, after: Energy },

    #[error(
        "decay step dropped more than λ allows: \
         before={before}, after={after}, \
         max_decay={max_decay} (epochs={epochs}, λ={half_life})"
    )]
    DecayExceededLambda {
        before: Energy,
        after: Energy,
        max_decay: Energy,
        epochs: u64,
        half_life: u64,
    },
}

pub struct ConservationCheck;

impl ConservationCheck {
    /// Audit a redirect-only step. The total must be preserved exactly.
    /// `before` and `after` are the accumulator totals on either side.
    pub fn redirect(
        before: &EnergyAccumulator,
        after: &EnergyAccumulator,
    ) -> Result<(), ConservationViolation> {
        let b = before.total();
        let a = after.total();
        if a != b {
            return Err(ConservationViolation::RedirectChangedTotal {
                before: b,
                after: a,
            });
        }
        Ok(())
    }

    /// Audit a decay-only step. Total may not increase; any drop must be
    /// consistent with applying the global λ to `before.total()` for
    /// `epochs_elapsed`.
    pub fn decay_step(
        before: &EnergyAccumulator,
        after: &EnergyAccumulator,
        epochs_elapsed: u64,
        lambda: ChainLambda,
    ) -> Result<(), ConservationViolation> {
        let b = before.total();
        let a = after.total();
        if a > b {
            return Err(ConservationViolation::DecayIncreasedTotal {
                before: b,
                after: a,
            });
        }
        // The maximum allowed *retained* total after λ-decay over the
        // elapsed epochs. We assert `after >= retained_min`, which is
        // `after >= energy_at_epoch(before, λ, epochs_elapsed)`.
        let retained_min = energy_at_epoch(b, lambda.half_life(), epochs_elapsed);
        if a < retained_min {
            // The drop (b - a) was greater than what λ allows.
            return Err(ConservationViolation::DecayExceededLambda {
                before: b,
                after: a,
                max_decay: b.saturating_sub(retained_min),
                epochs: epochs_elapsed,
                half_life: lambda.half_life(),
            });
        }
        Ok(())
    }

    /// Audit a composite block step that may include both redirects and
    /// `epochs_elapsed > 0`. Same bound as `decay_step` because redirects
    /// preserve the total — the only legal source of total drop is decay,
    /// regardless of how many redirects ran in between.
    pub fn block_step(
        before: &EnergyAccumulator,
        after: &EnergyAccumulator,
        epochs_elapsed: u64,
        lambda: ChainLambda,
    ) -> Result<(), ConservationViolation> {
        Self::decay_step(before, after, epochs_elapsed, lambda)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compartment::Compartment;
    use crate::lambda::Lambda;
    use crate::redirect::{EnergyRedirect, RedirectKind};

    fn lambda(epochs: u64) -> ChainLambda {
        ChainLambda::new(Lambda::from_epochs(epochs))
    }

    #[test]
    fn redirect_preserves_total() {
        let before = EnergyAccumulator::new(1_000, 500, 0, 0);
        let mut after = before;
        EnergyRedirect::new(RedirectKind::MevBurn, 75)
            .apply(&mut after)
            .unwrap();
        ConservationCheck::redirect(&before, &after).unwrap();
    }

    #[test]
    fn redirect_that_secretly_destroys_is_caught() {
        let before = EnergyAccumulator::new(1_000, 500, 0, 0);
        let mut after = before;
        // Direct mutation that simulates a bug: debit Stake without
        // crediting any other compartment. This is exactly the class of
        // bug the conservation invariant exists to catch.
        after.debit(Compartment::Stake, 200).unwrap();
        let err = ConservationCheck::redirect(&before, &after).unwrap_err();
        assert_eq!(
            err,
            ConservationViolation::RedirectChangedTotal {
                before: 1500,
                after: 1300,
            }
        );
    }

    #[test]
    fn decay_zero_epochs_is_no_op() {
        let acc = EnergyAccumulator::new(1_000, 0, 0, 0);
        ConservationCheck::decay_step(&acc, &acc, 0, lambda(100)).unwrap();
    }

    #[test]
    fn decay_full_halving_passes() {
        let before = EnergyAccumulator::new(1_000, 0, 0, 0);
        // After exactly one half-life, retained_min = 500. The chain may
        // hold any value in [500, 1000].
        let after_full = EnergyAccumulator::new(500, 0, 0, 0);
        ConservationCheck::decay_step(&before, &after_full, 100, lambda(100)).unwrap();
        // Holding more than necessary is also fine (decay isn't forced).
        let after_partial = EnergyAccumulator::new(750, 0, 0, 0);
        ConservationCheck::decay_step(&before, &after_partial, 100, lambda(100)).unwrap();
    }

    #[test]
    fn decay_increase_is_rejected() {
        let before = EnergyAccumulator::new(1_000, 0, 0, 0);
        let after = EnergyAccumulator::new(1_001, 0, 0, 0);
        let err = ConservationCheck::decay_step(&before, &after, 0, lambda(100)).unwrap_err();
        assert!(matches!(
            err,
            ConservationViolation::DecayIncreasedTotal { .. }
        ));
    }

    #[test]
    fn decay_below_lambda_floor_is_rejected() {
        let before = EnergyAccumulator::new(1_000, 0, 0, 0);
        // After 100 epochs at half-life=100, retained_min = 500. Holding
        // only 400 means we destroyed 100 more than λ allows → violation.
        let after_too_low = EnergyAccumulator::new(400, 0, 0, 0);
        let err =
            ConservationCheck::decay_step(&before, &after_too_low, 100, lambda(100)).unwrap_err();
        match err {
            ConservationViolation::DecayExceededLambda {
                before,
                after,
                epochs,
                half_life,
                ..
            } => {
                assert_eq!(before, 1000);
                assert_eq!(after, 400);
                assert_eq!(epochs, 100);
                assert_eq!(half_life, 100);
            }
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[test]
    fn block_step_redirect_plus_decay() {
        let before = EnergyAccumulator::new(1_000, 0, 0, 0);
        let mut after = before;
        // Redirect (preserves total)
        EnergyRedirect::new(RedirectKind::MevBurn, 100)
            .apply(&mut after)
            .unwrap();
        // Then decay one half-life uniformly across the accumulator —
        // simulate by halving the totals in each bucket.
        after = EnergyAccumulator::new(450, 0, 50, 0); // total=500
        ConservationCheck::block_step(&before, &after, 100, lambda(100)).unwrap();
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use crate::compartment::Compartment;
    use crate::lambda::Lambda;
    use crate::redirect::{EnergyRedirect, RedirectKind};
    use proptest::prelude::*;

    fn arb_redirect_kind() -> impl Strategy<Value = RedirectKind> {
        prop_oneof![
            Just(RedirectKind::Slash),
            Just(RedirectKind::SlashSettle),
            Just(RedirectKind::MevBurn),
            Just(RedirectKind::Demurrage),
            Just(RedirectKind::RefreshPayout),
        ]
    }

    proptest! {
        /// Property: a sequence of arbitrary redirects (those that succeed)
        /// preserves the total exactly, regardless of order.
        #[test]
        fn redirect_sequence_preserves_total(
            initial in (0u64..1_000_000, 0u64..1_000_000, 0u64..1_000_000, 0u64..1_000_000),
            redirects in proptest::collection::vec((arb_redirect_kind(), 0u64..50_000), 0..40),
        ) {
            let (a, s, r, sp) = initial;
            let mut acc = EnergyAccumulator::new(a, s, r, sp);
            let pre_total = acc.total();
            for (kind, amount) in redirects {
                // We don't care if individual redirects fail — we care
                // that successful ones preserve the total.
                let _ = EnergyRedirect::new(kind, amount).apply(&mut acc);
            }
            prop_assert_eq!(acc.total(), pre_total);
        }

        /// Property: a decay-only step never increases total, and never
        /// drops below the λ-bound.
        #[test]
        fn decay_step_within_lambda_floor(
            buckets in (0u64..10_000_000, 0u64..10_000_000, 0u64..10_000_000, 0u64..10_000_000),
            half_life in 1u64..1024,
            epochs in 0u64..2048,
            // Choose a "decay factor" in [retained_min/before, 1.0] applied uniformly.
            keep_promille in 500u64..=1000,
        ) {
            let (a, s, r, sp) = buckets;
            let before = EnergyAccumulator::new(a, s, r, sp);
            let lambda = ChainLambda::new(Lambda::from_epochs(half_life));
            let pre_total = before.total();
            let retained_min = energy_at_epoch(pre_total, half_life, epochs);
            // Pick an after_total in the legal interval [retained_min, pre_total].
            let span = pre_total.saturating_sub(retained_min);
            let drop = span.saturating_mul(1_000u64.saturating_sub(keep_promille)) / 1_000;
            let after_total = pre_total.saturating_sub(drop);
            // Allocate after_total entirely into Accounts to keep the
            // construction simple.
            let after = EnergyAccumulator::new(after_total, 0, 0, 0);
            let result = ConservationCheck::decay_step(&before, &after, epochs, lambda);
            prop_assert!(
                result.is_ok(),
                "expected ok, got {:?} (pre={}, post={}, retained_min={})",
                result, pre_total, after_total, retained_min
            );
            // Sanity: in particular, after.total() must be >= retained_min.
            prop_assert!(after.total() >= retained_min);
            // And the original Compartment enum is still usable, just to
            // anchor we haven't accidentally broken iteration somewhere.
            prop_assert_eq!(Compartment::ALL.len(), 4);
        }
    }
}
