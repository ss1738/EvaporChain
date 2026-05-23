//! Decaying engagement window — the attention-budget primitive.
//!
//! `EngagementWindow` accumulates engagement events with their own
//! decay rate. It's a moving-average-style attention quantity that
//! the chain enforces:
//!
//! - `register(epoch_now, weight)` adds `weight` to the current
//!   engagement, then re-anchors to `epoch_now`.
//! - `attention_at(epoch_now)` returns the decayed engagement at
//!   `epoch_now` per the standard half-life rule with
//!   `attention_half_life`.
//!
//! Crucially: yesterday's engagement evaporates. A token that was
//! adored last month and ignored since has a low attention score
//! today, exactly as if it had never been seen.

use evaporchain_types::{energy_at_epoch, Energy, Epoch, HalfLife};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EngagementWindowError {
    #[error("attention_half_life must be > 0")]
    ZeroHalfLife,
    #[error("register epoch {epoch} is before last anchor {last_anchor}")]
    BackwardsTime { epoch: Epoch, last_anchor: Epoch },
    #[error("engagement weight overflow")]
    Overflow,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngagementWindow {
    /// Cached attention at the most recent anchor (mint or register).
    pub cached_attention: Energy,
    /// Half-life of the attention quantity itself. 1 day worth of
    /// epochs is a sensible default; specifics live with the token.
    pub attention_half_life: HalfLife,
    /// Epoch the cached attention was last set.
    pub anchor_epoch: Epoch,
    /// Total cumulative engagement registered (audit trail; doesn't
    /// affect decay math). Useful for display + portable
    /// "lifetime engagement" attestations.
    pub lifetime_total: Energy,
}

impl EngagementWindow {
    pub fn new(
        attention_half_life: HalfLife,
        anchor_epoch: Epoch,
    ) -> Result<Self, EngagementWindowError> {
        if attention_half_life == 0 {
            return Err(EngagementWindowError::ZeroHalfLife);
        }
        Ok(Self {
            cached_attention: 0,
            attention_half_life,
            anchor_epoch,
            lifetime_total: 0,
        })
    }

    /// Decayed attention at `epoch_now`. Pure function.
    pub fn attention_at(&self, epoch_now: Epoch) -> Energy {
        let elapsed = epoch_now.saturating_sub(self.anchor_epoch);
        energy_at_epoch(self.cached_attention, self.attention_half_life, elapsed)
    }

    /// Register `weight` units of engagement at `epoch_now`. Decays
    /// the cached attention to `epoch_now` first, then adds the
    /// weight, then re-anchors. Errors on backward time or overflow.
    pub fn register(
        &mut self,
        epoch_now: Epoch,
        weight: Energy,
    ) -> Result<(), EngagementWindowError> {
        if epoch_now < self.anchor_epoch {
            return Err(EngagementWindowError::BackwardsTime {
                epoch: epoch_now,
                last_anchor: self.anchor_epoch,
            });
        }
        let now_attn = self.attention_at(epoch_now);
        let new = now_attn
            .checked_add(weight)
            .ok_or(EngagementWindowError::Overflow)?;
        self.cached_attention = new;
        self.anchor_epoch = epoch_now;
        self.lifetime_total = self.lifetime_total.saturating_add(weight);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero_half_life() {
        assert_eq!(
            EngagementWindow::new(0, 0).unwrap_err(),
            EngagementWindowError::ZeroHalfLife
        );
    }

    #[test]
    fn fresh_window_has_zero_attention() {
        let w = EngagementWindow::new(100, 0).unwrap();
        assert_eq!(w.attention_at(0), 0);
        assert_eq!(w.attention_at(1000), 0);
    }

    #[test]
    fn register_increments_attention() {
        let mut w = EngagementWindow::new(100, 0).unwrap();
        w.register(5, 50).unwrap();
        // At epoch 5 immediately after register, attention=50.
        assert_eq!(w.attention_at(5), 50);
        assert_eq!(w.lifetime_total, 50);
    }

    #[test]
    fn attention_decays_over_time() {
        let mut w = EngagementWindow::new(100, 0).unwrap();
        w.register(0, 1000).unwrap();
        // After 100 epochs (one half-life), ≈ 500 (with linear interp).
        let later = w.attention_at(100);
        assert!(later < 1000);
        assert!(later > 0);
    }

    #[test]
    fn yesterdays_likes_evaporate() {
        // Doctrine: "Yesterday's likes evaporate just like the token
        // they're trying to save." Concretely: if the only engagement
        // happened long ago, current attention is near zero.
        let mut w = EngagementWindow::new(10, 0).unwrap();
        w.register(0, 1000).unwrap();
        // After many half-lives of silence, attention → 0.
        assert_eq!(w.attention_at(10_000), 0);
    }

    #[test]
    fn register_at_same_epoch_compounds() {
        let mut w = EngagementWindow::new(100, 0).unwrap();
        w.register(5, 10).unwrap();
        w.register(5, 20).unwrap();
        w.register(5, 30).unwrap();
        // 0 elapsed since each register => attention is the sum.
        assert_eq!(w.attention_at(5), 60);
        assert_eq!(w.lifetime_total, 60);
    }

    #[test]
    fn register_decays_then_adds() {
        let mut w = EngagementWindow::new(100, 0).unwrap();
        w.register(0, 1000).unwrap();
        // After 100 epochs (one half-life), attention ≈ 500.
        // Register +1000 at epoch 100: new attention ≈ 1500.
        w.register(100, 1000).unwrap();
        let after = w.attention_at(100);
        assert!(
            (1400..=1600).contains(&after),
            "expected ≈1500, got {after}"
        );
        // Lifetime is the sum of all weights ever registered.
        assert_eq!(w.lifetime_total, 2000);
    }

    #[test]
    fn register_backwards_in_time_rejected() {
        let mut w = EngagementWindow::new(100, 50).unwrap();
        let err = w.register(10, 5).unwrap_err();
        assert!(matches!(err, EngagementWindowError::BackwardsTime { .. }));
    }

    #[test]
    fn overflow_rejected() {
        let mut w = EngagementWindow::new(100, 0).unwrap();
        // Saturate close to MAX, then register more.
        w.register(0, Energy::MAX - 5).unwrap();
        let err = w.register(0, 10).unwrap_err();
        assert_eq!(err, EngagementWindowError::Overflow);
    }

    #[test]
    fn round_trip_serde() {
        let mut w = EngagementWindow::new(100, 5).unwrap();
        w.register(10, 50).unwrap();
        let s = serde_json::to_string(&w).unwrap();
        let back: EngagementWindow = serde_json::from_str(&s).unwrap();
        assert_eq!(w, back);
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Attention is monotone non-increasing in time between
        /// registrations.
        #[test]
        fn attention_monotone_between_registers(
            half_life in 1u64..10_000,
            weight in 1u64..1_000_000,
            anchor in 0u64..1000,
            t_a in 0u64..2_000_000,
            extra in 0u64..1_000_000,
        ) {
            let mut w = EngagementWindow::new(half_life, anchor).unwrap();
            // Set attention by registering once.
            w.register(anchor, weight).unwrap();
            let a = w.attention_at(t_a.max(anchor));
            let b = w.attention_at(t_a.max(anchor).saturating_add(extra));
            prop_assert!(b <= a, "attention went up: {a} → {b}");
        }
    }
}
