//! Bucket classifier — pure function of (item, epoch_now, horizons).
//!
//! Buckets describe how soon the item will fall *below a death
//! threshold* (typically 1 unit of energy — i.e. effectively gone).
//! The wallet UI shows them as headline counts; validators agree on
//! the per-item bucket.
//!
//! Bucket boundaries:
//!
//! - **Decayed** — current energy is already at the death threshold.
//! - **Today** — will reach threshold within `horizon_today` epochs.
//! - **Tomorrow** — within `horizon_tomorrow` (must be > today).
//! - **ThisWeek** — within `horizon_week` (must be > tomorrow).
//! - **Healthy** — beyond `horizon_week`.

use evaporchain_types::{energy_at_epoch, Energy, Epoch};
use serde::{Deserialize, Serialize};

use crate::item::TriageItem;

/// Energy threshold below which an item is considered Decayed for
/// triage purposes. Default 1 — anything that rounds to zero.
pub const DEATH_THRESHOLD: Energy = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TriageBucket {
    Decayed,
    Today,
    Tomorrow,
    ThisWeek,
    Healthy,
}

/// Compute how many epochs from `epoch_now` until `item` reaches
/// `DEATH_THRESHOLD`. Returns `None` if the item is already at or
/// below threshold.
///
/// Closed-form: `e * 2^(-t / h) ≤ threshold` ⇒ `t ≥ h * log2(e / threshold)`.
/// Computed integer-only via repeated halving (fast for practical e).
pub fn epochs_until_threshold(item: &TriageItem, epoch_now: Epoch) -> Option<u64> {
    let now_e = item.energy_at(epoch_now);
    if now_e <= DEATH_THRESHOLD {
        return None;
    }
    // Find smallest k such that `energy_at_epoch(now_e, h, k) <= threshold`.
    // Energy halves every `h` epochs; ceil(log2(now_e / threshold)) half-lives.
    let mut hops = 0u64;
    let mut e = now_e;
    while e > DEATH_THRESHOLD {
        e = energy_at_epoch(e, item.half_life, item.half_life);
        hops += 1;
        if hops > 256 {
            // Defensive cap — for absurdly high energies, return a
            // very-far-away horizon rather than loop forever.
            return Some(u64::MAX / 2);
        }
    }
    Some(hops.saturating_mul(item.half_life))
}

/// Classify an item into a bucket using the supplied horizons.
pub fn classify(
    item: &TriageItem,
    epoch_now: Epoch,
    horizon_today: u64,
    horizon_tomorrow: u64,
    horizon_week: u64,
) -> TriageBucket {
    debug_assert!(horizon_today <= horizon_tomorrow);
    debug_assert!(horizon_tomorrow <= horizon_week);
    let until = match epochs_until_threshold(item, epoch_now) {
        None => return TriageBucket::Decayed,
        Some(t) => t,
    };
    if until <= horizon_today {
        TriageBucket::Today
    } else if until <= horizon_tomorrow {
        TriageBucket::Tomorrow
    } else if until <= horizon_week {
        TriageBucket::ThisWeek
    } else {
        TriageBucket::Healthy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(energy: Energy, half_life: u64, last_refreshed: u64) -> TriageItem {
        TriageItem::new([0; 32], energy, half_life, last_refreshed).unwrap()
    }

    #[test]
    fn fresh_item_is_healthy() {
        let it = item(1_000_000, 1000, 0);
        let b = classify(&it, 0, 1, 2, 7);
        assert_eq!(b, TriageBucket::Healthy);
    }

    #[test]
    fn already_dead_item_is_decayed() {
        // energy 1, half_life 1, 100 epochs later ⇒ 0.
        let it = item(1, 1, 0);
        let b = classify(&it, 100, 1, 2, 7);
        assert_eq!(b, TriageBucket::Decayed);
    }

    #[test]
    fn item_within_today_horizon_is_today() {
        // energy=2, half_life=1: at epoch_now=0, energy=2; one more
        // half-life ⇒ 1 (still > threshold? threshold is exclusive
        // i.e. <=1 is Decayed). Let's use slightly higher numbers.
        // energy=4, half_life=1: 4 → 2 → 1. Two half-lives to threshold.
        let it = item(4, 1, 0);
        let b = classify(&it, 0, 5, 10, 50);
        // until_threshold = 2 epochs ⇒ Today (≤ 5).
        assert_eq!(b, TriageBucket::Today);
    }

    #[test]
    fn buckets_progress_with_horizons() {
        // Same item; classify under different horizon configurations.
        let it = item(8, 1, 0);
        // Until threshold ≈ 3 epochs (8→4→2→1).
        assert_eq!(
            classify(&it, 0, 5, 10, 50),
            TriageBucket::Today,
            "until 3 ≤ today 5"
        );
        assert_eq!(
            classify(&it, 0, 1, 5, 50),
            TriageBucket::Tomorrow,
            "today 1 < until 3 ≤ tomorrow 5"
        );
        assert_eq!(
            classify(&it, 0, 1, 2, 5),
            TriageBucket::ThisWeek,
            "tomorrow 2 < until 3 ≤ week 5"
        );
        assert_eq!(
            classify(&it, 0, 1, 2, 2),
            TriageBucket::Healthy,
            "week 2 < until 3"
        );
    }

    #[test]
    fn epochs_until_threshold_is_none_when_at_threshold() {
        let it = item(1, 100, 0);
        assert!(epochs_until_threshold(&it, 0).is_none());
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Bucket monotonicity: as time passes, the same item moves
        /// only toward Decayed (never goes back to Healthy).
        #[test]
        fn bucket_monotone_in_time(
            energy in 2u64..1_000_000,
            half_life in 1u64..1000,
            t_a in 0u64..1_000_000,
            extra in 0u64..1_000_000,
        ) {
            let it = TriageItem::new([0; 32], energy, half_life, 0).unwrap();
            let order = |b: TriageBucket| match b {
                TriageBucket::Healthy => 0,
                TriageBucket::ThisWeek => 1,
                TriageBucket::Tomorrow => 2,
                TriageBucket::Today => 3,
                TriageBucket::Decayed => 4,
            };
            let b_a = classify(&it, t_a, 1, 2, 7);
            let b_b = classify(&it, t_a.saturating_add(extra), 1, 2, 7);
            prop_assert!(order(b_b) >= order(b_a));
        }
    }
}
