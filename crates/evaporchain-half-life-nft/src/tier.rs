//! `Tier` ladder — retention thresholds + per-tier half-life.

use serde::{Deserialize, Serialize};

/// One rung of the retention ladder.
///
/// `min_held_epochs`: cumulative held-time (since current holder
/// took possession) required to enter this tier.
/// `half_life_epochs`: while at this tier, energy halves every
/// `half_life_epochs` epochs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tier {
    pub min_held_epochs: u64,
    pub half_life_epochs: u64,
}

/// A retention ladder is a strictly-sorted list of tiers from
/// most-permissive to most-favourable (longer half-life as you
/// climb).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TierLadder {
    /// Sorted by `min_held_epochs` ascending. The first rung
    /// MUST have `min_held_epochs == 0` (everyone starts at tier 0).
    pub rungs: Vec<Tier>,
}

impl TierLadder {
    /// Construct, validating `rungs` is non-empty, starts at 0,
    /// is strictly increasing in `min_held_epochs`, and has
    /// non-decreasing `half_life_epochs`.
    pub fn new(rungs: Vec<Tier>) -> Option<Self> {
        if rungs.is_empty() {
            return None;
        }
        if rungs[0].min_held_epochs != 0 {
            return None;
        }
        for w in rungs.windows(2) {
            if w[0].min_held_epochs >= w[1].min_held_epochs {
                return None;
            }
            if w[0].half_life_epochs > w[1].half_life_epochs {
                // Half-life must be non-decreasing as you climb.
                return None;
            }
        }
        for r in &rungs {
            if r.half_life_epochs == 0 {
                return None;
            }
        }
        Some(Self { rungs })
    }

    /// Return the highest tier index whose `min_held_epochs ≤ held`.
    pub fn tier_index_for(&self, held_epochs: u64) -> usize {
        self.rungs
            .iter()
            .rposition(|t| t.min_held_epochs <= held_epochs)
            .unwrap_or(0)
    }

    pub fn tier_at(&self, index: usize) -> &Tier {
        &self.rungs[index.min(self.rungs.len() - 1)]
    }
}

/// Default 5-tier ladder. Each tier roughly doubles the half-life.
///
/// | Tier | Min held epochs | Half-life epochs |
/// |---|---|---|
/// | 0 | 0      | 100  |
/// | 1 | 1_000  | 200  |
/// | 2 | 5_000  | 400  |
/// | 3 | 20_000 | 800  |
/// | 4 | 50_000 | 1600 |
pub fn default_ladder() -> TierLadder {
    TierLadder::new(vec![
        Tier {
            min_held_epochs: 0,
            half_life_epochs: 100,
        },
        Tier {
            min_held_epochs: 1_000,
            half_life_epochs: 200,
        },
        Tier {
            min_held_epochs: 5_000,
            half_life_epochs: 400,
        },
        Tier {
            min_held_epochs: 20_000,
            half_life_epochs: 800,
        },
        Tier {
            min_held_epochs: 50_000,
            half_life_epochs: 1600,
        },
    ])
    .expect("default ladder is well-formed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_ladder() {
        assert!(TierLadder::new(vec![]).is_none());
    }

    #[test]
    fn rejects_first_rung_not_zero() {
        let bad = vec![Tier {
            min_held_epochs: 100,
            half_life_epochs: 100,
        }];
        assert!(TierLadder::new(bad).is_none());
    }

    #[test]
    fn rejects_non_increasing_held() {
        let bad = vec![
            Tier {
                min_held_epochs: 0,
                half_life_epochs: 100,
            },
            Tier {
                min_held_epochs: 0,
                half_life_epochs: 200,
            },
        ];
        assert!(TierLadder::new(bad).is_none());
    }

    #[test]
    fn rejects_decreasing_half_life() {
        // Climbing should make half-life longer, not shorter.
        let bad = vec![
            Tier {
                min_held_epochs: 0,
                half_life_epochs: 200,
            },
            Tier {
                min_held_epochs: 100,
                half_life_epochs: 100,
            },
        ];
        assert!(TierLadder::new(bad).is_none());
    }

    #[test]
    fn rejects_zero_half_life() {
        let bad = vec![Tier {
            min_held_epochs: 0,
            half_life_epochs: 0,
        }];
        assert!(TierLadder::new(bad).is_none());
    }

    #[test]
    fn default_ladder_is_well_formed() {
        let l = default_ladder();
        assert_eq!(l.rungs.len(), 5);
        assert_eq!(l.rungs[0].min_held_epochs, 0);
    }

    #[test]
    fn tier_index_picks_correct_rung() {
        let l = default_ladder();
        assert_eq!(l.tier_index_for(0), 0);
        assert_eq!(l.tier_index_for(999), 0);
        assert_eq!(l.tier_index_for(1_000), 1);
        assert_eq!(l.tier_index_for(4_999), 1);
        assert_eq!(l.tier_index_for(5_000), 2);
        assert_eq!(l.tier_index_for(19_999), 2);
        assert_eq!(l.tier_index_for(20_000), 3);
        assert_eq!(l.tier_index_for(49_999), 3);
        assert_eq!(l.tier_index_for(50_000), 4);
        assert_eq!(l.tier_index_for(u64::MAX), 4);
    }

    #[test]
    fn round_trip_serde() {
        let l = default_ladder();
        let s = serde_json::to_string(&l).unwrap();
        let back: TierLadder = serde_json::from_str(&s).unwrap();
        assert_eq!(l, back);
    }
}
