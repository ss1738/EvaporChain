//! `MortisMonitor` — tracks the consecutive-epochs run below `ε`.

use serde::{Deserialize, Serialize};

use evaporchain_types::Energy;

use crate::condition::MortisCondition;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MortisMonitor {
    pub condition: MortisCondition,
    /// Consecutive epochs the refresh pool has been at or below floor.
    pub consecutive_below: u64,
    /// Latest epoch observed by `tick`.
    pub latest_epoch: u64,
    /// Whether the death trigger has fired (latched once true).
    pub triggered: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TickOutcome {
    /// Pool above floor; consecutive_below reset to 0.
    Healthy,
    /// Pool at-or-below floor; consecutive_below incremented; not yet triggered.
    Counting { consecutive_below: u64 },
    /// Trigger fired this tick.
    JustTriggered,
    /// Trigger already fired earlier; subsequent ticks are no-ops.
    AlreadyTriggered,
}

impl MortisMonitor {
    pub const fn new(condition: MortisCondition) -> Self {
        Self {
            condition,
            consecutive_below: 0,
            latest_epoch: 0,
            triggered: false,
        }
    }

    /// Advance one observation. `current_epoch` must be ≥
    /// `self.latest_epoch` (monotone) — past observations are
    /// silently dropped.
    pub fn tick(&mut self, current_epoch: u64, refresh_pool_total: Energy) -> TickOutcome {
        if self.triggered {
            self.latest_epoch = current_epoch.max(self.latest_epoch);
            return TickOutcome::AlreadyTriggered;
        }
        if current_epoch < self.latest_epoch {
            return TickOutcome::Healthy; // ignored past tick
        }
        self.latest_epoch = current_epoch;
        if refresh_pool_total > self.condition.refresh_pool_floor {
            self.consecutive_below = 0;
            return TickOutcome::Healthy;
        }
        self.consecutive_below = self.consecutive_below.saturating_add(1);
        if self.consecutive_below >= self.condition.sustained_epochs {
            self.triggered = true;
            TickOutcome::JustTriggered
        } else {
            TickOutcome::Counting {
                consecutive_below: self.consecutive_below,
            }
        }
    }

    pub fn is_triggered(&self) -> bool {
        self.triggered
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cond(floor: Energy, n: u64) -> MortisCondition {
        MortisCondition::new(floor, n)
    }

    #[test]
    fn fresh_monitor_not_triggered() {
        let m = MortisMonitor::new(cond(1000, 10));
        assert!(!m.is_triggered());
    }

    #[test]
    fn pool_above_floor_resets_count() {
        let mut m = MortisMonitor::new(cond(1000, 10));
        let o = m.tick(1, 500);
        assert_eq!(
            o,
            TickOutcome::Counting {
                consecutive_below: 1
            }
        );
        let o = m.tick(2, 2000);
        assert_eq!(o, TickOutcome::Healthy);
        assert_eq!(m.consecutive_below, 0);
    }

    #[test]
    fn sustained_breach_triggers() {
        let mut m = MortisMonitor::new(cond(1000, 3));
        m.tick(1, 500);
        m.tick(2, 500);
        let o = m.tick(3, 500);
        assert_eq!(o, TickOutcome::JustTriggered);
        assert!(m.is_triggered());
    }

    #[test]
    fn trigger_is_latched() {
        let mut m = MortisMonitor::new(cond(1000, 2));
        m.tick(1, 0);
        m.tick(2, 0); // triggers
        let o = m.tick(3, 999_999); // would be healthy if not latched
        assert_eq!(o, TickOutcome::AlreadyTriggered);
        assert!(m.is_triggered());
    }

    #[test]
    fn pool_at_floor_counts_as_below() {
        // Floor = 1000; pool at exactly 1000 is NOT strictly above → counts.
        let mut m = MortisMonitor::new(cond(1000, 2));
        let o = m.tick(1, 1000);
        assert!(matches!(o, TickOutcome::Counting { .. }));
    }

    #[test]
    fn past_epoch_tick_ignored() {
        let mut m = MortisMonitor::new(cond(1000, 2));
        m.tick(10, 500);
        let o = m.tick(5, 500); // earlier — ignored
                                // consecutive_below stays at 1, not 2
        assert_eq!(m.consecutive_below, 1);
        assert_eq!(o, TickOutcome::Healthy);
    }
}
