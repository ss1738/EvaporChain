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
