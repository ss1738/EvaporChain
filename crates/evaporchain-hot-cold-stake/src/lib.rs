//! Hot/Cold Stake (Tier 2).
//!
//! Per `research/INVENTION_STACK.md` §4.2:
//!
//! > **Hot/Cold Stake** — Two-temperature equilibrium per validator;
//! > novel split-temperature stake.
//!
//! ## Mechanics
//!
//! Each validator holds two pools:
//!
//! - **Hot stake** — actively used in block production, decays at the
//!   "hot" half-life (short, e.g. 100 epochs). Earns refresh credit
//!   on every block.
//! - **Cold stake** — held in reserve, decays at the "cold" half-life
//!   (long, e.g. 10 000 epochs). Doesn't participate but doesn't need
//!   refreshing.
//!
//! Validators can `promote` cold→hot (instant) and `demote` hot→cold
//! (subject to a chain-set cool-down). Slash and reward apply to hot
//! stake; cold stake is the long-term skin in the game.
//!
//! ## Module map
//!
//! - [`stake`] — `HotColdStake { hot, cold, hot_lambda, cold_lambda,
//!   last_touched_epoch }`. promote / demote / hot_decay / cold_decay.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use evaporchain_energy_kernel::{energy_at_epoch, ChainLambda};
use evaporchain_types::Energy;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotColdStake {
    pub hot: Energy,
    pub cold: Energy,
    pub hot_lambda: ChainLambda,
    pub cold_lambda: ChainLambda,
    pub last_touched_epoch: u64,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum StakeError {
    #[error("not enough cold stake to promote: have {have}, need {need}")]
    InsufficientCold { have: Energy, need: Energy },
    #[error("not enough hot stake to demote: have {have}, need {need}")]
    InsufficientHot { have: Energy, need: Energy },
}

impl HotColdStake {
    pub const fn new(
        hot: Energy,
        cold: Energy,
        hot_lambda: ChainLambda,
        cold_lambda: ChainLambda,
        last_touched_epoch: u64,
    ) -> Self {
        Self {
            hot,
            cold,
            hot_lambda,
            cold_lambda,
            last_touched_epoch,
        }
    }

    /// Apply both decays from `last_touched_epoch` to `current_epoch`.
    pub fn decay(self, current_epoch: u64) -> Self {
        if current_epoch <= self.last_touched_epoch {
            return self;
        }
        let elapsed = current_epoch - self.last_touched_epoch;
        let hot_after = energy_at_epoch(self.hot, self.hot_lambda.half_life(), elapsed);
        let cold_after = energy_at_epoch(self.cold, self.cold_lambda.half_life(), elapsed);
        Self {
            hot: hot_after,
            cold: cold_after,
            hot_lambda: self.hot_lambda,
            cold_lambda: self.cold_lambda,
            last_touched_epoch: current_epoch,
        }
    }

    /// Move `amount` from cold → hot (instant).
    pub fn promote(mut self, amount: Energy) -> Result<Self, StakeError> {
        if self.cold < amount {
            return Err(StakeError::InsufficientCold {
                have: self.cold,
                need: amount,
            });
        }
        self.cold -= amount;
        self.hot = self.hot.saturating_add(amount);
        Ok(self)
    }

    /// Move `amount` from hot → cold. Cool-down enforcement is the
    /// caller's job (substrate doesn't track time-of-demotion).
    pub fn demote(mut self, amount: Energy) -> Result<Self, StakeError> {
        if self.hot < amount {
            return Err(StakeError::InsufficientHot {
                have: self.hot,
                need: amount,
            });
        }
        self.hot -= amount;
        self.cold = self.cold.saturating_add(amount);
        Ok(self)
    }

    /// Total = hot + cold. Saturating.
    pub fn total(self) -> Energy {
        self.hot.saturating_add(self.cold)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_energy_kernel::Lambda;

    fn fresh() -> HotColdStake {
        HotColdStake::new(
            1000,
            10_000,
            ChainLambda::new(Lambda::from_epochs(100)),
            ChainLambda::new(Lambda::from_epochs(10_000)),
            0,
        )
    }

    #[test]
    fn fresh_total_is_hot_plus_cold() {
        let s = fresh();
        assert_eq!(s.total(), 11_000);
    }

    #[test]
    fn hot_decays_faster_than_cold() {
        let s = fresh();
        let after = s.decay(100);
        // Hot at half-life 100 → 1000 → 500.
        // Cold at half-life 10_000 → 10_000 → almost unchanged.
        assert_eq!(after.hot, 500);
        assert!(after.cold > 9_900);
    }

    #[test]
    fn promote_moves_cold_to_hot() {
        let s = fresh();
        let after = s.promote(2000).unwrap();
        assert_eq!(after.hot, 3000);
        assert_eq!(after.cold, 8000);
    }

    #[test]
    fn promote_rejected_when_insufficient_cold() {
        let s = fresh();
        let err = s.promote(100_000).unwrap_err();
        assert!(matches!(err, StakeError::InsufficientCold { .. }));
    }

    #[test]
    fn demote_moves_hot_to_cold() {
        let s = fresh();
        let after = s.demote(500).unwrap();
        assert_eq!(after.hot, 500);
        assert_eq!(after.cold, 10_500);
    }

    #[test]
    fn demote_rejected_when_insufficient_hot() {
        let s = fresh();
        let err = s.demote(10_000).unwrap_err();
        assert!(matches!(err, StakeError::InsufficientHot { .. }));
    }

    #[test]
    fn no_time_elapsed_decay_no_op() {
        let s = fresh();
        assert_eq!(s.decay(0), s);
    }
}
