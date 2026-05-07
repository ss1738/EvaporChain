//! Inbox aggregator — turns a slice of items + an epoch into the
//! headline-counts the wallet UI renders.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::bucket::{classify, TriageBucket};
use crate::item::TriageItem;

/// Headline counts per bucket. Wallet renders "3 items decay today /
/// Tomorrow (7) / This Week (24) / Healthy (137)" directly from this.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inbox {
    pub today: usize,
    pub tomorrow: usize,
    pub this_week: usize,
    pub healthy: usize,
    pub decayed: usize,
}

impl Inbox {
    pub fn total(&self) -> usize {
        self.today + self.tomorrow + self.this_week + self.healthy + self.decayed
    }
}

/// Build an `Inbox` from the items + horizons. Pure; deterministic;
/// no I/O.
pub fn bucket_counts(
    items: &[TriageItem],
    epoch_now: u64,
    horizon_today: u64,
    horizon_tomorrow: u64,
    horizon_week: u64,
) -> Inbox {
    let mut counts: HashMap<TriageBucket, usize> = HashMap::new();
    for it in items {
        let b = classify(it, epoch_now, horizon_today, horizon_tomorrow, horizon_week);
        *counts.entry(b).or_insert(0) += 1;
    }
    Inbox {
        today: *counts.get(&TriageBucket::Today).unwrap_or(&0),
        tomorrow: *counts.get(&TriageBucket::Tomorrow).unwrap_or(&0),
        this_week: *counts.get(&TriageBucket::ThisWeek).unwrap_or(&0),
        healthy: *counts.get(&TriageBucket::Healthy).unwrap_or(&0),
        decayed: *counts.get(&TriageBucket::Decayed).unwrap_or(&0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(energy: u64, half_life: u64, last_refreshed: u64) -> TriageItem {
        TriageItem::new([0; 32], energy, half_life, last_refreshed).unwrap()
    }

    #[test]
    fn empty_inbox_is_zero() {
        let inbox = bucket_counts(&[], 0, 1, 2, 7);
        assert_eq!(inbox, Inbox::default());
        assert_eq!(inbox.total(), 0);
    }

    #[test]
    fn mixed_inbox_classifies_each() {
        // Hit Today, Healthy, and Decayed simultaneously.
        // - Today: tiny energy, half_life=1 ⇒ ~3 epochs to threshold
        //   (8 → 4 → 2 → 1 → 0). horizon_today=5 ⇒ falls in Today.
        // - Healthy: big energy, big half_life ⇒ many epochs until
        //   threshold. With energy=1_000_000 and half_life=1000, the
        //   first halving alone takes 1000 epochs ≫ horizon_week=100.
        // - Decayed: energy=1 with stale last_refreshed ⇒ already
        //   sub-threshold at the read epoch.
        let items = vec![
            item(8, 1, 100),            // refreshed at 100; ~3 epochs to threshold
            item(1_000_000, 1000, 100), // healthy — first halving after 1000 epochs
            item(1, 100, 0),            // decayed — already at the death floor
        ];
        let inbox = bucket_counts(&items, 100, 5, 10, 100);
        assert_eq!(inbox.today, 1);
        assert_eq!(inbox.healthy, 1);
        assert_eq!(inbox.decayed, 1);
        assert_eq!(inbox.total(), 3);
    }

    #[test]
    fn round_trip_serde() {
        let inbox = Inbox {
            today: 3,
            tomorrow: 7,
            this_week: 24,
            healthy: 137,
            decayed: 0,
        };
        let s = serde_json::to_string(&inbox).unwrap();
        let back: Inbox = serde_json::from_str(&s).unwrap();
        assert_eq!(inbox, back);
        // The doctrine numbers from §A5.4 round-trip cleanly.
        assert_eq!(back.today, 3);
        assert_eq!(back.tomorrow, 7);
    }
}
