//! Evaporating Conviction Vote — governance with decay-bounded patience.
//!
//! ## Standard conviction voting (Commons Stack 2018)
//!
//! Voters allocate `stake` to a proposal. Conviction integrates
//! over time toward an asymptote of `stake / (1 - α)` (where α
//! is the conviction-growth parameter, typically α = 0.9 per
//! tick). A proposal passes when its accumulated conviction
//! exceeds a `threshold`. Voters can re-allocate at any time;
//! conviction reverts toward zero on withdrawal.
//!
//! Implication: standard conviction voting requires a voter to
//! "sit on" a proposal patiently for the conviction to build —
//! a Schelling point against governance flash-mobs.
//!
//! ## What "evaporating" adds
//!
//! In standard conviction, the voter's `stake` is fixed once
//! deposited. In the Evaporating variant, **the stake itself
//! decays** under the chain's single-λ. The conviction integrand
//! becomes:
//!
//!   c(t+1) = α · c(t) + (1 - α) · stake(t)
//!
//! where `stake(t)` is the energy-decayed stake at tick `t`.
//!
//! Implication: conviction has TWO time scales —
//! - Conviction *integration*: grows with time (faster with high α).
//! - Stake *decay*: shrinks with time (faster with high λ).
//!
//! When `λ > (1 - α)`, the decay outpaces the integration: a
//! voter who deposits-and-leaves sees their conviction peak and
//! then *decline*. A proposal that requires sustained conviction
//! requires sustained engagement (re-staking) to keep its
//! supporters' weight fresh.
//!
//! ## Three structural decisions enforced as tests
//!
//! 1. **Pure-integer fixed-point.** Conviction in micros (1.0 =
//!    10⁶ micros); α as a fixed-point ratio out of 1_000_000.
//!    Validator-deterministic byte-equality.
//!
//! 2. **Bounded asymptote with decay-aware ceiling.** The
//!    conviction at time t is bounded above by `stake(t) /
//!    (1 - α)`. With `stake(t)` decaying, the asymptote shrinks.
//!
//! 3. **Pass-then-stay-passed only via threshold-at-tick.**
//!    Conviction passing the threshold mints a one-shot pass
//!    event; subsequent decay below threshold doesn't un-pass
//!    (the higher layer's effect was applied). This prevents
//!    pass/un-pass flapping.
//!
//! ## What this crate does NOT do
//!
//! - It does NOT model the actual proposal payload. The chain
//!   wraps proposal data; this crate tracks the conviction state.
//! - It does NOT perform the chain's decay tick. Caller passes
//!   the energy-adjusted stake at each `tick` call.
//! - It does NOT enforce stake-uniqueness across proposals. A
//!   voter is free to allocate to multiple proposals; the chain's
//!   higher layer manages "total stake ≤ voter balance".
//!
//! ## Module map
//!
//! - [`proposal`] — [`Proposal`] state machine.
//! - [`registry`] — [`ConvictionRegistry`] for per-proposal +
//!   per-voter tracking.

pub mod proposal;
pub mod registry;

pub use proposal::{Proposal, ProposalError, ProposalId, ALPHA_MICROS_DEFAULT, MICROS};
pub use registry::{ConvictionRegistry, RegistryError, VoterId};

#[cfg(test)]
mod press_claim_tests {
    use super::*;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "Evaporating Conviction Vote uses pure-integer
    /// fixed-point conviction (no f64). Sustained stake makes
    /// conviction grow toward the threshold; once Passed, it
    /// STAYS passed even if subsequent decay drops conviction
    /// below threshold (no flapping). Non-monotone ticks rejected;
    /// alpha out of (0, MICROS) range rejected at construction."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let id = ProposalId([1u8; 32]);
        // Threshold below the asymptote (10×stake = 100_000_000) so
        // it's reached in finite ticks.
        let mut p = Proposal::new(id, ALPHA_MICROS_DEFAULT, 50_000_000, 0).unwrap();
        assert!(!p.is_passed());

        // Sustained stake — conviction grows.
        let mut last_conviction = 0u128;
        for t in 1u64..50 {
            p.tick(t, 10_000_000).unwrap();
            assert!(
                p.conviction_micros >= last_conviction,
                "conviction monotone with constant stake"
            );
            last_conviction = p.conviction_micros;
        }
        // After enough ticks, threshold is crossed.
        assert!(p.is_passed());
        let passed_conviction = p.conviction_micros;

        // Stake decays to 0; conviction now decreases — but Passed
        // status is sticky (no flapping back to Pending).
        for t in 50u64..200 {
            p.tick(t, 0).unwrap();
        }
        assert!(p.is_passed(), "Passed status must be sticky");
        assert!(
            p.conviction_micros < passed_conviction,
            "decay shrinks conviction"
        );

        // Non-monotone tick rejected.
        assert!(matches!(
            p.tick(100, 0),
            Err(ProposalError::NonMonotoneTick { .. })
        ));

        // Construction guards: alpha must be in (0, MICROS).
        assert!(matches!(
            Proposal::new(id, 0, 100, 0),
            Err(ProposalError::InvalidAlpha(0))
        ));
        assert!(matches!(
            Proposal::new(id, MICROS as u64, 100, 0),
            Err(ProposalError::InvalidAlpha(_))
        ));
        // Zero threshold rejected.
        assert!(matches!(
            Proposal::new(id, ALPHA_MICROS_DEFAULT, 0, 0),
            Err(ProposalError::InvalidThreshold)
        ));
    }
}
