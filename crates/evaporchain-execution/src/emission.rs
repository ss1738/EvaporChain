//! Block-reward emission schedule — closes the substrate piece of
//! `AUDIT_2026_05_06.md` MEDIUM (Block reward / emission schedule).
//!
//! Audit note: "Not implemented; mainnet token economics undefined."
//!
//! The pre-fix codebase had a flat `ValidatorEconConfig.block_reward:
//! u64` constant and a hard-coded value in genesis. There was no
//! schedule shape — no halving, no linear decay, no terminal supply
//! cap. Mainnet operators couldn't pin down "what's the inflation
//! curve?" without code-diving.
//!
//! This module ships the schedule TYPE plus pure-function reward
//! computation. Three families are supported:
//!
//!   1. `Constant`     — flat block reward forever (today's
//!      behaviour, made explicit).
//!   2. `Halving`      — Bitcoin-style halving every N epochs.
//!      `initial_reward >> (epoch / hl)`,
//!      saturating to 0 at 64 halvings.
//!   3. `LinearDecay`  — linear interpolation from
//!      `initial_reward` at epoch 0 to 0 at
//!      `decay_epochs`. Floors at 0
//!      thereafter.
//!
//! Plus an optional `max_supply: Option<u128>` cap that overrides
//! the per-block reward to 0 once cumulative emissions reach the
//! cap. The cap is checked against `cumulative_emitted` supplied
//! by the caller, so this module stays stateless.
//!
//! # Hot-path wiring
//!
//! This commit lands the type + tests. Wiring into
//! `process_block_rewards` (called from `lib.rs:3061` and
//! `parallel.rs:2056`) is the follow-on: the executor needs to
//! track `cumulative_emitted` in StateDB and call
//! `block_reward_at` per block. That's a state-machine change
//! with its own blast radius, deliberately separated.
//!
//! # Numeric representation
//!
//! Per-block reward stays `u64` to match the existing
//! `ValidatorEconConfig.block_reward` field shape. Cumulative
//! emitted is `u128` because over a chain's lifetime
//! `blocks × block_reward` can exceed u64 (10M blocks/year ×
//! 1000 reward = 10^10/year; over 10 years = 10^11, still fits
//! u64, but adding a max_supply cap of e.g. 100M tokens × 10^18
//! base units = 10^26 needs u128).

// Type definitions + pure-function `block_reward_at` moved to
// `evaporchain-types::emission` 2026-05-07 so `Tokenomics` can carry
// `Option<EmissionParams>` directly. Re-exported here to keep this
// crate's public API stable.
pub use evaporchain_types::emission::{
    block_reward_at, EmissionError, EmissionParams, EmissionSchedule,
};

#[cfg(test)]
mod tests {
    use super::*;

    // ── construction validation ──────────────────────────────────

    #[test]
    fn constant_with_positive_reward_constructs() {
        let p = EmissionParams::new(100, EmissionSchedule::Constant, None);
        assert!(p.is_ok());
    }

    #[test]
    fn zero_initial_reward_rejected() {
        let err = EmissionParams::new(0, EmissionSchedule::Constant, None).unwrap_err();
        assert_eq!(err, EmissionError::ZeroInitialReward);
    }

    #[test]
    fn halving_with_zero_period_rejected() {
        let err = EmissionParams::new(
            100,
            EmissionSchedule::Halving {
                epochs_per_halving: 0,
            },
            None,
        )
        .unwrap_err();
        assert_eq!(err, EmissionError::ZeroHalvingPeriod);
    }

    #[test]
    fn linear_decay_with_zero_window_rejected() {
        let err = EmissionParams::new(100, EmissionSchedule::LinearDecay { decay_epochs: 0 }, None)
            .unwrap_err();
        assert_eq!(err, EmissionError::ZeroDecayWindow);
    }

    // ── Constant schedule ────────────────────────────────────────

    #[test]
    fn constant_returns_initial_reward_at_every_epoch() {
        let p = EmissionParams::new(100, EmissionSchedule::Constant, None).unwrap();
        assert_eq!(block_reward_at(&p, 0, 0), 100);
        assert_eq!(block_reward_at(&p, 1_000_000, 0), 100);
        assert_eq!(block_reward_at(&p, u64::MAX, 0), 100);
    }

    // ── Halving schedule ─────────────────────────────────────────

    #[test]
    fn halving_halves_at_each_period_boundary() {
        let p = EmissionParams::new(
            1024,
            EmissionSchedule::Halving {
                epochs_per_halving: 100,
            },
            None,
        )
        .unwrap();
        assert_eq!(block_reward_at(&p, 0, 0), 1024);
        assert_eq!(block_reward_at(&p, 99, 0), 1024); // before first halving
        assert_eq!(block_reward_at(&p, 100, 0), 512); // first halving
        assert_eq!(block_reward_at(&p, 200, 0), 256); // second
        assert_eq!(block_reward_at(&p, 300, 0), 128); // third
        assert_eq!(block_reward_at(&p, 1000, 0), 1); // 10 halvings: 1024 >> 10 = 1
    }

    #[test]
    fn halving_saturates_at_zero_after_64_halvings() {
        let p = EmissionParams::new(
            1024,
            EmissionSchedule::Halving {
                epochs_per_halving: 1,
            },
            None,
        )
        .unwrap();
        assert_eq!(block_reward_at(&p, 64, 0), 0);
        assert_eq!(block_reward_at(&p, 1_000_000, 0), 0);
    }

    // ── LinearDecay schedule ─────────────────────────────────────

    #[test]
    fn linear_decay_starts_at_initial_and_decays_to_zero() {
        let p = EmissionParams::new(
            1000,
            EmissionSchedule::LinearDecay { decay_epochs: 1000 },
            None,
        )
        .unwrap();
        assert_eq!(block_reward_at(&p, 0, 0), 1000);
        assert_eq!(block_reward_at(&p, 250, 0), 750); // 75% remaining
        assert_eq!(block_reward_at(&p, 500, 0), 500); // halfway
        assert_eq!(block_reward_at(&p, 750, 0), 250); // 25% remaining
                                                      // At epoch == decay_epochs, the explicit zero branch kicks in.
        assert_eq!(block_reward_at(&p, 1000, 0), 0);
        assert_eq!(block_reward_at(&p, 999, 0), 1); // last block before zero
        assert_eq!(block_reward_at(&p, 10_000, 0), 0); // far past window
    }

    // ── max_supply cap ───────────────────────────────────────────

    #[test]
    fn max_supply_zero_emissions_after_cap() {
        let p = EmissionParams::new(100, EmissionSchedule::Constant, Some(1_000_000)).unwrap();
        assert_eq!(block_reward_at(&p, 0, 999_999), 1); // headroom = 1
        assert_eq!(block_reward_at(&p, 0, 1_000_000), 0); // exactly at cap
        assert_eq!(block_reward_at(&p, 0, 2_000_000), 0); // beyond cap
    }

    #[test]
    fn max_supply_clips_final_block_to_headroom() {
        // Cap at 1050; constant reward 100. At 1000 cumulative,
        // headroom = 50, so reward should be clipped from 100 to 50.
        let p = EmissionParams::new(100, EmissionSchedule::Constant, Some(1050)).unwrap();
        assert_eq!(block_reward_at(&p, 0, 1000), 50);
    }

    #[test]
    fn no_max_supply_means_unbounded() {
        let p = EmissionParams::new(100, EmissionSchedule::Constant, None).unwrap();
        // Even at huge cumulative, reward is the full schedule
        // value.
        assert_eq!(block_reward_at(&p, 0, u128::MAX / 2), 100);
    }

    // ── serde round-trip (genesis-config integration) ────────────

    #[test]
    fn emission_params_serde_round_trip_constant() {
        let p = EmissionParams::new(100, EmissionSchedule::Constant, Some(1_000_000)).unwrap();
        let json = serde_json::to_string(&p).unwrap();
        let p2: EmissionParams = serde_json::from_str(&json).unwrap();
        assert_eq!(p, p2);
    }

    #[test]
    fn emission_params_serde_round_trip_halving() {
        let p = EmissionParams::new(
            1024,
            EmissionSchedule::Halving {
                epochs_per_halving: 210_000,
            },
            None,
        )
        .unwrap();
        let json = serde_json::to_string(&p).unwrap();
        let p2: EmissionParams = serde_json::from_str(&json).unwrap();
        assert_eq!(p, p2);
    }

    #[test]
    fn emission_params_serde_round_trip_linear_decay() {
        let p = EmissionParams::new(
            1000,
            EmissionSchedule::LinearDecay {
                decay_epochs: 10_000_000,
            },
            Some(10_000_000_000),
        )
        .unwrap();
        let json = serde_json::to_string(&p).unwrap();
        let p2: EmissionParams = serde_json::from_str(&json).unwrap();
        assert_eq!(p, p2);
    }

    // ── press claim ──────────────────────────────────────────────

    #[test]
    fn press_claim_three_schedules_with_cap_compose_correctly() {
        // Bitcoin-style halving every 210K blocks, capped at 21M.
        let halving = EmissionParams::new(
            50,
            EmissionSchedule::Halving {
                epochs_per_halving: 210_000,
            },
            Some(21_000_000),
        )
        .unwrap();
        assert_eq!(block_reward_at(&halving, 0, 0), 50);
        assert_eq!(block_reward_at(&halving, 210_000, 0), 25);

        // Linear decay over 10M epochs (10-year sunset).
        let linear = EmissionParams::new(
            1000,
            EmissionSchedule::LinearDecay {
                decay_epochs: 10_000_000,
            },
            None,
        )
        .unwrap();
        assert_eq!(block_reward_at(&linear, 5_000_000, 0), 500);

        // Flat with cap.
        let flat = EmissionParams::new(100, EmissionSchedule::Constant, Some(1_000)).unwrap();
        assert_eq!(block_reward_at(&flat, 0, 950), 50); // clipped to headroom
        assert_eq!(block_reward_at(&flat, 0, 1000), 0); // post-cap
    }
}
