//! Qualitative streak strength bucket.
//!
//! Two thresholds, three buckets: a streak is `Healthy` (≥ 0.75 of
//! capacity), `Aging` (0.40–0.74), or `Broken` (< 0.40). The wallet
//! nudges at Aging — *before* the user loses everything — and only
//! triggers the "you broke your streak" cliff at Broken.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StrengthBucket {
    /// Strength ≥ 0.75 — full health
    Healthy,
    /// 0.40 ≤ strength < 0.75 — wallet should nudge
    Aging,
    /// Strength < 0.40 — streak considered broken
    Broken,
}

/// Threshold below which the streak transitions Healthy → Aging.
pub const AGING_THRESHOLD_BP: u32 = 75; // 0.75 in basis points / 100
/// Threshold below which the streak transitions Aging → Broken.
pub const BROKEN_THRESHOLD_BP: u32 = 40; // 0.40

/// Map a strength fraction in [0.0, 1.0] to its qualitative bucket.
/// Uses integer-only basis-point comparison so validators agree
/// without floating-point divergence.
pub fn bucket_for_strength(strength: f64) -> StrengthBucket {
    let bp = (strength * 100.0).clamp(0.0, 100.0) as u32;
    if bp >= AGING_THRESHOLD_BP {
        StrengthBucket::Healthy
    } else if bp >= BROKEN_THRESHOLD_BP {
        StrengthBucket::Aging
    } else {
        StrengthBucket::Broken
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_strength_is_healthy() {
        assert_eq!(bucket_for_strength(1.0), StrengthBucket::Healthy);
        assert_eq!(bucket_for_strength(0.99), StrengthBucket::Healthy);
        assert_eq!(bucket_for_strength(0.75), StrengthBucket::Healthy);
    }

    #[test]
    fn middle_strength_is_aging() {
        assert_eq!(bucket_for_strength(0.74), StrengthBucket::Aging);
        assert_eq!(bucket_for_strength(0.50), StrengthBucket::Aging);
        assert_eq!(bucket_for_strength(0.40), StrengthBucket::Aging);
    }

    #[test]
    fn low_strength_is_broken() {
        assert_eq!(bucket_for_strength(0.39), StrengthBucket::Broken);
        assert_eq!(bucket_for_strength(0.10), StrengthBucket::Broken);
        assert_eq!(bucket_for_strength(0.0), StrengthBucket::Broken);
    }

    #[test]
    fn out_of_range_clamps() {
        assert_eq!(bucket_for_strength(-1.0), StrengthBucket::Broken);
        assert_eq!(bucket_for_strength(2.0), StrengthBucket::Healthy);
    }

    #[test]
    fn round_trip_serde() {
        for b in [
            StrengthBucket::Healthy,
            StrengthBucket::Aging,
            StrengthBucket::Broken,
        ] {
            let s = serde_json::to_string(&b).unwrap();
            let back: StrengthBucket = serde_json::from_str(&s).unwrap();
            assert_eq!(b, back);
        }
    }
}
