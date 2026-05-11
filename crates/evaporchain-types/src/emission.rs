//! Block-reward emission schedule.
//!
//! Originally landed in `crates/evaporchain-execution/src/emission.rs`
//! to address `AUDIT_2026_05_06.md` MEDIUM (Block reward / emission
//! schedule). Moved here 2026-05-07 so [`crate::genesis::Tokenomics`]
//! can carry an `Option<EmissionParams>` directly without inverting
//! the dep graph (execution depends on types, not vice versa).
//!
//! `evaporchain-execution::emission` now re-exports these types for
//! backwards compatibility.
//!
//! Three schedule shapes:
//!
//!   1. `Constant`     — flat block reward forever.
//!   2. `Halving`      — Bitcoin-style halving every N epochs.
//!      `initial_reward >> (epoch / hl)`, saturating
//!      to 0 at 64 halvings.
//!   3. `LinearDecay`  — linear interpolation from `initial_reward`
//!      at epoch 0 to 0 at `decay_epochs`. Floors
//!      at 0 thereafter.
//!
//! Plus an optional `max_supply: Option<u128>` cap that overrides
//! the per-block reward to 0 once cumulative emissions reach the
//! cap. Stateless — caller supplies `cumulative_emitted` per call.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Shape of the per-block reward over time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum EmissionSchedule {
    /// Reward is `initial_reward` every block, forever (subject
    /// to `max_supply` cap if any).
    Constant,
    /// Reward halves every `epochs_per_halving` epochs.
    /// `initial_reward >> (epoch / hl)`, saturating to 0 at 64
    /// halvings (which would underflow u64). Bitcoin-style.
    Halving { epochs_per_halving: u64 },
    /// Linear interpolation from `initial_reward` at epoch 0 to
    /// `0` at `decay_epochs`. Floors at 0 thereafter.
    LinearDecay { decay_epochs: u64 },
}

/// Genesis-encodable emission parameters. The chain serialises
/// this in `genesis-mainnet.json` so operators can audit the
/// schedule before launch and validators converge on the same
/// reward trajectory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmissionParams {
    /// Reward at epoch 0. Per-block units; matches existing
    /// `ValidatorEconConfig.block_reward`.
    pub initial_reward: u64,
    /// Shape of the schedule.
    pub schedule: EmissionSchedule,
    /// Optional cap on cumulative emitted tokens. Once reached,
    /// `block_reward_at` returns 0 regardless of schedule. `None`
    /// = uncapped (chain emits forever per the schedule).
    pub max_supply: Option<u128>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EmissionError {
    /// `Halving { epochs_per_halving: 0 }` would divide by zero.
    #[error("Halving schedule requires epochs_per_halving > 0")]
    ZeroHalvingPeriod,
    /// `LinearDecay { decay_epochs: 0 }` would divide by zero.
    #[error("LinearDecay schedule requires decay_epochs > 0")]
    ZeroDecayWindow,
    /// `initial_reward = 0` is technically valid (no emissions
    /// at all), but explicit rejection helps catch genesis-config
    /// typos. Operators who genuinely want zero emissions should
    /// set `max_supply: Some(0)` or omit the schedule entirely.
    #[error("initial_reward = 0 — set max_supply: Some(0) for explicit no-emission chains")]
    ZeroInitialReward,
}

impl EmissionParams {
    /// Construct + validate. Rejects schedules with division-by-
    /// zero parameters or zero initial reward.
    pub fn new(
        initial_reward: u64,
        schedule: EmissionSchedule,
        max_supply: Option<u128>,
    ) -> Result<Self, EmissionError> {
        if initial_reward == 0 {
            return Err(EmissionError::ZeroInitialReward);
        }
        match schedule {
            EmissionSchedule::Constant => {}
            EmissionSchedule::Halving { epochs_per_halving } => {
                if epochs_per_halving == 0 {
                    return Err(EmissionError::ZeroHalvingPeriod);
                }
            }
            EmissionSchedule::LinearDecay { decay_epochs } => {
                if decay_epochs == 0 {
                    return Err(EmissionError::ZeroDecayWindow);
                }
            }
        }
        Ok(Self {
            initial_reward,
            schedule,
            max_supply,
        })
    }

    /// Provisional default — flat 100 tokens/block, no cap.
    pub const fn provisional_default() -> Self {
        Self {
            initial_reward: 100,
            schedule: EmissionSchedule::Constant,
            max_supply: None,
        }
    }
}

/// Compute the per-block reward at `epoch` given the schedule
/// and the cumulative-emitted total (used for `max_supply` cap
/// enforcement). Pure function; the chain calls this from the
/// reward-accumulator path with `cumulative_emitted` read from
/// `RewardAccumulator.total_minted`.
pub fn block_reward_at(params: &EmissionParams, epoch: u64, cumulative_emitted: u128) -> u64 {
    // Cap check — if we've hit max_supply, no further emissions.
    if let Some(cap) = params.max_supply {
        if cumulative_emitted >= cap {
            return 0;
        }
    }

    let raw = match params.schedule {
        EmissionSchedule::Constant => params.initial_reward,
        EmissionSchedule::Halving { epochs_per_halving } => {
            let halvings = epoch / epochs_per_halving;
            if halvings >= 64 {
                0
            } else {
                params.initial_reward >> halvings
            }
        }
        EmissionSchedule::LinearDecay { decay_epochs } => {
            if epoch >= decay_epochs {
                0
            } else {
                let remaining = (decay_epochs - epoch) as u128;
                let scaled = (params.initial_reward as u128).saturating_mul(remaining)
                    / (decay_epochs as u128);
                scaled as u64
            }
        }
    };

    // Cap the reward so cumulative_emitted + raw <= max_supply.
    if let Some(cap) = params.max_supply {
        let headroom = cap.saturating_sub(cumulative_emitted);
        if (raw as u128) > headroom {
            headroom as u64
        } else {
            raw
        }
    } else {
        raw
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── EmissionParams::new validation ────────────────────────────────

    #[test]
    fn new_constant_schedule_accepts_initial_reward() {
        let p = EmissionParams::new(1000, EmissionSchedule::Constant, None).unwrap();
        assert_eq!(p.initial_reward, 1000);
        assert!(matches!(p.schedule, EmissionSchedule::Constant));
        assert!(p.max_supply.is_none());
    }

    #[test]
    fn new_halving_schedule_accepts_positive_period() {
        let p = EmissionParams::new(
            1000,
            EmissionSchedule::Halving { epochs_per_halving: 100 },
            Some(1_000_000),
        )
        .unwrap();
        assert_eq!(p.max_supply, Some(1_000_000));
        assert!(
            matches!(p.schedule, EmissionSchedule::Halving { epochs_per_halving: 100 })
        );
    }

    #[test]
    fn new_linear_decay_accepts_positive_window() {
        let p = EmissionParams::new(
            1000,
            EmissionSchedule::LinearDecay { decay_epochs: 10_000 },
            None,
        )
        .unwrap();
        assert!(
            matches!(p.schedule, EmissionSchedule::LinearDecay { decay_epochs: 10_000 })
        );
    }

    #[test]
    fn new_rejects_zero_initial_reward() {
        let err = EmissionParams::new(0, EmissionSchedule::Constant, None).unwrap_err();
        assert_eq!(err, EmissionError::ZeroInitialReward);
    }

    #[test]
    fn new_rejects_zero_halving_period() {
        let err = EmissionParams::new(
            1000,
            EmissionSchedule::Halving { epochs_per_halving: 0 },
            None,
        )
        .unwrap_err();
        assert_eq!(err, EmissionError::ZeroHalvingPeriod);
    }

    #[test]
    fn new_rejects_zero_decay_window() {
        let err = EmissionParams::new(
            1000,
            EmissionSchedule::LinearDecay { decay_epochs: 0 },
            None,
        )
        .unwrap_err();
        assert_eq!(err, EmissionError::ZeroDecayWindow);
    }

    #[test]
    fn provisional_default_shape() {
        let p = EmissionParams::provisional_default();
        assert_eq!(p.initial_reward, 100);
        assert!(matches!(p.schedule, EmissionSchedule::Constant));
        assert!(p.max_supply.is_none());
    }

    // ─── block_reward_at: Constant schedule ────────────────────────────

    #[test]
    fn block_reward_constant_returns_initial_at_any_epoch() {
        let p = EmissionParams::new(500, EmissionSchedule::Constant, None).unwrap();
        assert_eq!(block_reward_at(&p, 0, 0), 500);
        assert_eq!(block_reward_at(&p, 1_000, 0), 500);
        assert_eq!(block_reward_at(&p, u64::MAX, 0), 500);
    }

    // ─── block_reward_at: Halving schedule ─────────────────────────────

    #[test]
    fn block_reward_halving_halves_at_each_period() {
        let p = EmissionParams::new(
            1024,
            EmissionSchedule::Halving { epochs_per_halving: 100 },
            None,
        )
        .unwrap();
        assert_eq!(block_reward_at(&p, 0, 0), 1024);
        assert_eq!(block_reward_at(&p, 99, 0), 1024);
        assert_eq!(block_reward_at(&p, 100, 0), 512);
        assert_eq!(block_reward_at(&p, 199, 0), 512);
        assert_eq!(block_reward_at(&p, 200, 0), 256);
        assert_eq!(block_reward_at(&p, 300, 0), 128);
        assert_eq!(block_reward_at(&p, 1_000, 0), 1);
    }

    #[test]
    fn block_reward_halving_returns_zero_after_64_halvings() {
        let p = EmissionParams::new(
            1_000_000,
            EmissionSchedule::Halving { epochs_per_halving: 1 },
            None,
        )
        .unwrap();
        // 64 halvings of any u64 saturates to 0 (right-shift past 63).
        assert_eq!(block_reward_at(&p, 64, 0), 0);
        assert_eq!(block_reward_at(&p, 1_000, 0), 0);
    }

    // ─── block_reward_at: LinearDecay schedule ─────────────────────────

    #[test]
    fn block_reward_linear_decay_linearly_decreases() {
        let p = EmissionParams::new(
            1000,
            EmissionSchedule::LinearDecay { decay_epochs: 100 },
            None,
        )
        .unwrap();
        // epoch 0: full reward
        assert_eq!(block_reward_at(&p, 0, 0), 1000);
        // epoch 50 (half-way): half reward
        assert_eq!(block_reward_at(&p, 50, 0), 500);
        // epoch 99 (one before end): tiny reward
        assert_eq!(block_reward_at(&p, 99, 0), 10);
        // epoch 100 (at end): zero
        assert_eq!(block_reward_at(&p, 100, 0), 0);
        // far past end: still zero
        assert_eq!(block_reward_at(&p, 10_000, 0), 0);
    }

    // ─── block_reward_at: max_supply cap enforcement ──────────────────

    #[test]
    fn block_reward_max_supply_returns_zero_when_cap_reached() {
        let p = EmissionParams::new(100, EmissionSchedule::Constant, Some(1_000)).unwrap();
        // Cumulative == cap → zero.
        assert_eq!(block_reward_at(&p, 0, 1_000), 0);
        // Cumulative past cap → still zero.
        assert_eq!(block_reward_at(&p, 0, 1_500), 0);
    }

    #[test]
    fn block_reward_max_supply_clips_to_headroom_at_boundary() {
        // 1000 cap; 950 already emitted. Schedule wants to emit 100,
        // but only 50 headroom remains → block_reward_at returns 50.
        let p = EmissionParams::new(100, EmissionSchedule::Constant, Some(1_000)).unwrap();
        assert_eq!(block_reward_at(&p, 0, 950), 50);
    }

    #[test]
    fn block_reward_max_supply_full_when_room_exceeds_schedule() {
        // 10_000 cap; 100 emitted. Plenty of room.
        let p = EmissionParams::new(100, EmissionSchedule::Constant, Some(10_000)).unwrap();
        assert_eq!(block_reward_at(&p, 0, 100), 100);
    }

    #[test]
    fn block_reward_no_cap_returns_schedule_value() {
        let p = EmissionParams::new(100, EmissionSchedule::Constant, None).unwrap();
        // Any cumulative_emitted is fine when there's no cap.
        assert_eq!(block_reward_at(&p, 0, u128::MAX), 100);
    }

    #[test]
    fn block_reward_halving_with_cap_clips_correctly() {
        let p = EmissionParams::new(
            1000,
            EmissionSchedule::Halving { epochs_per_halving: 100 },
            Some(1_500),
        )
        .unwrap();
        // After halving once: schedule wants 500. 1_400 already emitted
        // → only 100 headroom.
        assert_eq!(block_reward_at(&p, 100, 1_400), 100);
    }

    // ─── Error display strings — exercise the thiserror impls ─────────

    #[test]
    fn emission_error_display_strings() {
        let e1 = EmissionError::ZeroHalvingPeriod;
        assert!(e1.to_string().contains("Halving"));
        let e2 = EmissionError::ZeroDecayWindow;
        assert!(e2.to_string().contains("LinearDecay"));
        let e3 = EmissionError::ZeroInitialReward;
        assert!(e3.to_string().contains("initial_reward"));
    }
}
