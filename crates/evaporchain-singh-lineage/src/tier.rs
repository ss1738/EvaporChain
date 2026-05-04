//! Dormancy ladder — the *graduated* part of graduated inheritance.
//!
//! A `Ladder` is a sorted list of `DormancyTier` thresholds. As the
//! issuer's silence crosses each threshold, the successors' authority
//! shares accrue.
//!
//! Doctrine example: `[Tier(90d, 0.25), Tier(180d, 0.50), Tier(365d, 1.00)]`.
//! "If I'm silent 90 days, my daughter's key gains 25% authority;
//! 180 days, 50%; 365 days, full."
//!
//! Ladder invariants:
//! - **Strictly monotone in `epochs_dormant`** — each tier's
//!   threshold > the previous tier's threshold.
//! - **Non-decreasing in `authority_share`** — accruing authority
//!   never reverses.
//! - **`authority_share ∈ [0, 1]`** stored as basis points (×10000)
//!   so the math stays integer-only and validators agree without
//!   floating-point divergence.

use evaporchain_types::Epoch;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Authority share in basis points (1 bp = 0.01%). 10_000 = 1.00.
pub const FULL_AUTHORITY_BP: u32 = 10_000;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LadderError {
    #[error("ladder must contain at least one tier")]
    Empty,
    #[error("epochs_dormant {epochs} appears more than once or out of order")]
    NonMonotoneThreshold { epochs: Epoch },
    #[error("authority_share decreased between tiers ({prev} → {curr})")]
    AuthorityDecreased { prev: u32, curr: u32 },
    #[error("authority_share {bp} exceeds FULL_AUTHORITY_BP ({FULL_AUTHORITY_BP})")]
    AuthorityAboveOne { bp: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DormancyTier {
    /// Epochs of issuer silence required to reach this tier.
    pub epochs_dormant: Epoch,
    /// Authority share *up to and including* this tier, in basis
    /// points (10_000 = 1.00). Cumulative; a tier's share replaces
    /// the previous tier's share when reached.
    pub authority_share_bp: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ladder {
    pub tiers: Vec<DormancyTier>,
}

impl Ladder {
    /// Construct a ladder, validating monotonicity invariants.
    pub fn new(tiers: Vec<DormancyTier>) -> Result<Self, LadderError> {
        if tiers.is_empty() {
            return Err(LadderError::Empty);
        }
        let mut prev_epochs: Option<Epoch> = None;
        let mut prev_share: u32 = 0;
        for t in &tiers {
            if t.authority_share_bp > FULL_AUTHORITY_BP {
                return Err(LadderError::AuthorityAboveOne {
                    bp: t.authority_share_bp,
                });
            }
            if let Some(prev) = prev_epochs {
                if t.epochs_dormant <= prev {
                    return Err(LadderError::NonMonotoneThreshold {
                        epochs: t.epochs_dormant,
                    });
                }
            }
            if t.authority_share_bp < prev_share {
                return Err(LadderError::AuthorityDecreased {
                    prev: prev_share,
                    curr: t.authority_share_bp,
                });
            }
            prev_epochs = Some(t.epochs_dormant);
            prev_share = t.authority_share_bp;
        }
        Ok(Self { tiers })
    }

    /// Authority share (basis points) at `dormancy_epochs`. Looks up
    /// the highest tier whose threshold has been crossed.
    pub fn share_at(&self, dormancy_epochs: u64) -> u32 {
        let mut share = 0u32;
        for t in &self.tiers {
            if dormancy_epochs >= t.epochs_dormant {
                share = t.authority_share_bp;
            } else {
                break;
            }
        }
        share
    }

    /// Doctrine convenience: `[(90d, 25%), (180d, 50%), (365d, 100%)]`
    /// expressed in caller-supplied epochs-per-day.
    pub fn doctrine_default(epochs_per_day: u64) -> Result<Self, LadderError> {
        Self::new(vec![
            DormancyTier {
                epochs_dormant: 90 * epochs_per_day,
                authority_share_bp: 2_500,
            },
            DormancyTier {
                epochs_dormant: 180 * epochs_per_day,
                authority_share_bp: 5_000,
            },
            DormancyTier {
                epochs_dormant: 365 * epochs_per_day,
                authority_share_bp: FULL_AUTHORITY_BP,
            },
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_ladder() {
        let err = Ladder::new(vec![]).unwrap_err();
        assert_eq!(err, LadderError::Empty);
    }

    #[test]
    fn rejects_non_monotone_thresholds() {
        let err = Ladder::new(vec![
            DormancyTier { epochs_dormant: 100, authority_share_bp: 2_500 },
            DormancyTier { epochs_dormant: 50, authority_share_bp: 5_000 },
        ])
        .unwrap_err();
        assert!(matches!(err, LadderError::NonMonotoneThreshold { .. }));
    }

    #[test]
    fn rejects_duplicate_thresholds() {
        let err = Ladder::new(vec![
            DormancyTier { epochs_dormant: 100, authority_share_bp: 2_500 },
            DormancyTier { epochs_dormant: 100, authority_share_bp: 5_000 },
        ])
        .unwrap_err();
        assert!(matches!(err, LadderError::NonMonotoneThreshold { .. }));
    }

    #[test]
    fn rejects_decreasing_authority() {
        let err = Ladder::new(vec![
            DormancyTier { epochs_dormant: 100, authority_share_bp: 5_000 },
            DormancyTier { epochs_dormant: 200, authority_share_bp: 2_500 },
        ])
        .unwrap_err();
        assert!(matches!(err, LadderError::AuthorityDecreased { .. }));
    }

    #[test]
    fn rejects_authority_above_full() {
        let err = Ladder::new(vec![DormancyTier {
            epochs_dormant: 100,
            authority_share_bp: 20_000,
        }])
        .unwrap_err();
        assert!(matches!(err, LadderError::AuthorityAboveOne { .. }));
    }

    #[test]
    fn share_at_returns_highest_crossed_tier() {
        let ladder = Ladder::new(vec![
            DormancyTier { epochs_dormant: 90, authority_share_bp: 2_500 },
            DormancyTier { epochs_dormant: 180, authority_share_bp: 5_000 },
            DormancyTier { epochs_dormant: 365, authority_share_bp: 10_000 },
        ])
        .unwrap();
        assert_eq!(ladder.share_at(0), 0);
        assert_eq!(ladder.share_at(89), 0);
        assert_eq!(ladder.share_at(90), 2_500);
        assert_eq!(ladder.share_at(179), 2_500);
        assert_eq!(ladder.share_at(180), 5_000);
        assert_eq!(ladder.share_at(364), 5_000);
        assert_eq!(ladder.share_at(365), 10_000);
        assert_eq!(ladder.share_at(99_999), 10_000);
    }

    #[test]
    fn doctrine_default_matches_quote() {
        // Doctrine: "If I'm silent 90 days, my daughter's key gains
        // 25% authority; 180 days, 50%; 365 days, full."
        let ladder = Ladder::doctrine_default(1).unwrap(); // 1 epoch == 1 day
        assert_eq!(ladder.share_at(89), 0);
        assert_eq!(ladder.share_at(90), 2_500); // 25%
        assert_eq!(ladder.share_at(180), 5_000); // 50%
        assert_eq!(ladder.share_at(365), 10_000); // 100% — full
    }

    #[test]
    fn round_trip_serde() {
        let ladder = Ladder::doctrine_default(1).unwrap();
        let s = serde_json::to_string(&ladder).unwrap();
        let back: Ladder = serde_json::from_str(&s).unwrap();
        assert_eq!(ladder, back);
    }
}
