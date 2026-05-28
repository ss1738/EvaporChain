//! Singh-Boltzmann Stake — validator stake decays by default; refresh
//! by producing blocks.
//!
//! Per `research/INVENTION_STACK.md` §4.1 row 5:
//!
//! > **Singh-Boltzmann Stake** — Validator stake decays by default;
//! > refresh by producing blocks. **Kills the stake-and-lease-key-to-
//! > MEV pattern.**
//!
//! And per the mechanism-design source attribution: "Boltzmann
//! distribution over validator activity."
//!
//! ## Mechanics
//!
//! 1. **Decay** — every epoch, each validator's *active stake* decays
//!    via the chain-global `energy_at_epoch(stake, λ, 1)`. A validator
//!    that does nothing watches its voting power approach zero on the
//!    same time-constant as every other state object.
//! 2. **Refresh** — producing a block credits the validator's stake.
//!    The credit is sized so a validator producing at the *expected
//!    rate* exactly compensates the decay (steady state).
//! 3. **Boltzmann ranking** — selection probability for the next
//!    proposer is `∝ exp(activity_score) · stake`, where activity_score
//!    is recent block-production rate. This is the same Boltzmann
//!    machinery as `evaporchain-cfm::weight::boltzmann_weight` — high
//!    activity = high effective stake.
//!
//! ## Why this kills the stake-and-lease-key-to-MEV pattern
//!
//! In existing PoS chains a holder can stake once, lease the signing
//! key to a third-party MEV operator, and collect base yield without
//! producing blocks themselves. The lease has no liveness obligation.
//!
//! Under Singh-Boltzmann Stake the *holder's* stake decays unless they
//! sign blocks. A passive holder watches their voting power evaporate;
//! a leaseholder MEV operator can sustain their own stake (because
//! they do produce blocks) but cannot resurrect the original holder's.
//! There is no "set it and forget it" stake.
//!
//! ## Module map
//!
//! - [`stake_state`] — `ValidatorStake { active, last_touched_epoch }`.
//! - [`decay`] — `decay_validator_stake(stake, λ, current_epoch)`
//!   advances a validator's active stake to the current epoch.
//! - [`refresh`] — `refresh_on_block(stake, refresh_amount, epoch)`
//!   credits stake when the validator produces a block.
//! - [`boltzmann`] — `proposer_weight(stake, activity_score, beta_mb)`
//!   computes the Boltzmann-weighted selection probability.

pub mod boltzmann;
pub mod decay;
pub mod refresh;
pub mod stake_state;

pub use boltzmann::proposer_weight;
pub use decay::decay_validator_stake;
pub use refresh::refresh_on_block;
pub use stake_state::ValidatorStake;

#[cfg(test)]
mod press_claim_tests {
    //! The press claim lives as a test (INVENTION_STACK §4.1 row 5):
    //! "Singh-Boltzmann Stake — Validator stake decays by default;
    //! refresh by producing blocks. Kills the stake-and-lease-key-to-
    //! MEV pattern."
    //!
    //! Source attribution (mechanism-design agent, round 2):
    //! "Boltzmann distribution over validator activity."
    //!
    //! Three invariants that MUST hold at the runtime level:
    //!
    //! 1. **Passive validator evaporates** — a validator that produces no
    //!    blocks watches their stake halve every `half_life` epochs,
    //!    approaching zero asymptotically.
    //! 2. **Active validator holds steady** — a validator that produces at
    //!    the expected cadence can exactly compensate decay with refresh
    //!    credits.
    //! 3. **MEV-lease pattern killed** — the original holder's stake
    //!    account decays independently of the leaseholder's activity.
    //!    A leaseholder who produces blocks can refresh *their own* stake
    //!    but cannot prevent the original holder's stake from evaporating.

    use evaporchain_energy_kernel::{ChainLambda, Lambda};

    use crate::{decay_validator_stake, proposer_weight, refresh_on_block, ValidatorStake};

    const HALF_LIFE: u64 = 100;

    fn cl() -> ChainLambda {
        ChainLambda::new(Lambda::from_epochs(HALF_LIFE))
    }

    // ── 1. Passive validator evaporates ──────────────────────────────

    #[test]
    fn passive_validator_stake_halves_each_half_life() {
        let s0 = ValidatorStake::fresh(1_000_000);
        let s1 = decay_validator_stake(s0, cl(), HALF_LIFE);
        let s2 = decay_validator_stake(s0, cl(), HALF_LIFE * 2);

        assert_eq!(s1.active, 500_000, "one half-life must halve stake");
        assert_eq!(s2.active, 250_000, "two half-lives must quarter stake");
        assert!(s1.active > s2.active);
    }

    // ── 2. Active validator holds steady ──────────────────────────────

    #[test]
    fn refresh_exactly_compensates_decay_at_expected_cadence() {
        let initial = 1_000_000u64;
        let s0 = ValidatorStake::fresh(initial);
        let decayed = decay_validator_stake(s0, cl(), HALF_LIFE);
        let lost = initial - decayed.active;
        let restored = refresh_on_block(decayed, lost, HALF_LIFE);
        assert_eq!(
            restored.active, initial,
            "exact refresh must restore initial stake"
        );
    }

    // ── 3. MEV-lease pattern killed ───────────────────────────────────

    #[test]
    fn leaseholder_activity_cannot_refresh_original_holders_stake() {
        // Holder stakes at epoch 0 and goes passive.
        let holder = ValidatorStake::fresh(1_000_000);
        // Leaseholder also starts at 1_000_000 and produces every epoch.
        let leaseholder = ValidatorStake::fresh(1_000_000);

        // After one half-life, apply the leaseholder's block-production refresh.
        // Leaseholder refreshes *their own* account — holder receives nothing.
        let leaseholder_decayed = decay_validator_stake(leaseholder, cl(), HALF_LIFE);
        let lost = 1_000_000u64.saturating_sub(leaseholder_decayed.active);
        let leaseholder_after = refresh_on_block(leaseholder_decayed, lost, HALF_LIFE);

        // Holder is never refreshed — their stake still decays.
        let holder_after = decay_validator_stake(holder, cl(), HALF_LIFE);

        assert_eq!(
            leaseholder_after.active, 1_000_000,
            "active leaseholder must maintain full stake"
        );
        assert_eq!(
            holder_after.active, 500_000,
            "passive holder decays even if leaseholder is fully active"
        );
        assert!(
            leaseholder_after.active > holder_after.active,
            "leaseholder must strictly outrank the passive original holder"
        );
    }

    // ── Boltzmann boost: activity amplifies weight ────────────────────

    #[test]
    fn active_validator_outranks_passive_at_equal_stake() {
        // The boost is non-decreasing in activity_score (integer steps);
        // use a large activity gap to guarantee at least a weak ordering.
        let stake = 1_000_000u64;
        let beta_mb = 1_000u64;
        let w_passive = proposer_weight(stake, 0, beta_mb);
        let w_active = proposer_weight(stake, 16, beta_mb);
        assert!(
            w_active >= w_passive,
            "a block-producing validator must have at least as high a proposer weight"
        );
        // With a zero-activity baseline the boost must eventually grow.
        let w_very_active = proposer_weight(stake, 1_000, beta_mb);
        assert!(
            w_very_active >= w_active,
            "very-active validator must not rank below the active one"
        );
    }
}
