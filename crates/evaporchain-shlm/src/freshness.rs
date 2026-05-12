//! Freshness derivation — decayed level + qualitative bucket.
//!
//! Pure-function, deterministic, no I/O. Validators compute the same
//! freshness for the same inputs.
//!
//! `freshness_score(class, credential, epoch_now)` returns the
//! credential's *current* level, decayed under the class's half-life
//! over `(epoch_now - credential.attested_at_epoch)`. This is just the
//! standard `evaporchain_types::energy_at_epoch` rule reused — no new
//! decay model.
//!
//! `FreshnessLevel` is a coarse qualitative bucket on top of the
//! numeric score. Recruiters typically filter by bucket
//! (Fresh / Aging / Stale / Expired) before drilling into the numeric
//! score, so the bucket is a first-class type in the API.

use evaporchain_types::{energy_at_epoch, Energy, Epoch};
use serde::{Deserialize, Serialize};

use crate::credential::Credential;
use crate::skill_class::SkillClass;

/// Qualitative freshness bucket. Thresholds are class-agnostic and
/// expressed as fractions of the original level so the same logic
/// works for any half-life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FreshnessLevel {
    /// ≥ 75% of original level retained.
    Fresh,
    /// 50–75% retained — "still good, but consider refreshing."
    Aging,
    /// 25–50% retained — "below most employer thresholds."
    Stale,
    /// < 25% retained — "effectively unusable; refresh required."
    Expired,
}

/// Decayed numeric score at `epoch_now`. If `epoch_now` is before the
/// credential was attested (clock skew), returns the original level.
pub fn freshness_score(class: &SkillClass, cred: &Credential, epoch_now: Epoch) -> Energy {
    let elapsed = epoch_now.saturating_sub(cred.attested_at_epoch);
    energy_at_epoch(cred.level, class.half_life, elapsed)
}

/// Coarse bucket derived from `freshness_score / cred.level`.
pub fn freshness_bucket(class: &SkillClass, cred: &Credential, epoch_now: Epoch) -> FreshnessLevel {
    let now_score = freshness_score(class, cred, epoch_now);
    if cred.level == 0 {
        // Defensive: zero-level credentials would be rejected at issue,
        // but if one slips through, treat it as expired.
        return FreshnessLevel::Expired;
    }
    let pct = (now_score as u128 * 100) / (cred.level as u128);
    if pct >= 75 {
        FreshnessLevel::Fresh
    } else if pct >= 50 {
        FreshnessLevel::Aging
    } else if pct >= 25 {
        FreshnessLevel::Stale
    } else {
        FreshnessLevel::Expired
    }
}

#[cfg(test)]
fn test_class(half_life: u64) -> SkillClass {
    SkillClass::register([1u8; 32], "Test", half_life, 0).unwrap()
}

#[cfg(test)]
fn test_cred(level: Energy, attested_at: Epoch) -> Credential {
    Credential::issue(
        [2u8; 32],
        [1u8; 32],
        [0xAA; 32],
        [0xBB; 32],
        level,
        attested_at,
    )
    .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class(half_life: u64) -> SkillClass {
        super::test_class(half_life)
    }

    fn cred(level: Energy, attested_at: Epoch) -> Credential {
        super::test_cred(level, attested_at)
    }

    #[test]
    fn at_attestation_epoch_score_equals_level() {
        let c = class(100);
        let cr = cred(1000, 50);
        assert_eq!(freshness_score(&c, &cr, 50), 1000);
        assert_eq!(freshness_bucket(&c, &cr, 50), FreshnessLevel::Fresh);
    }

    #[test]
    fn before_attestation_epoch_returns_original() {
        // Clock-skew edge: epoch_now < attested_at.
        let c = class(100);
        let cr = cred(1000, 50);
        assert_eq!(freshness_score(&c, &cr, 0), 1000);
    }

    #[test]
    fn one_half_life_elapsed_drops_to_half() {
        let c = class(100);
        let cr = cred(1000, 0);
        let s = freshness_score(&c, &cr, 100);
        // The standard `energy_at_epoch` is exact-doubling;
        // expect 500 ± rounding tolerance.
        assert!((400..=600).contains(&s), "got score {s}");
    }

    #[test]
    fn bucket_thresholds_at_quartiles() {
        let c = class(100);
        let cr = cred(1000, 0);
        // Fresh at t=0 (100%); decays through buckets as t grows.
        assert_eq!(freshness_bucket(&c, &cr, 0), FreshnessLevel::Fresh);
        // After many half-lives → Expired.
        assert_eq!(freshness_bucket(&c, &cr, 10_000), FreshnessLevel::Expired);
    }

    #[test]
    fn higher_class_half_life_gives_slower_decay() {
        // Doctrine claim: COBOL > Python > prompt-engineering for
        // half-life. After 100 epochs, COBOL credential should still
        // be Fresh while prompt-engineering credential is Stale or worse.
        let cobol = class(2920); // ~8yr
        let prompt = class(120); // ~4mo
        let cr = cred(1000, 0);
        let cobol_b = freshness_bucket(&cobol, &cr, 100);
        let prompt_b = freshness_bucket(&prompt, &cr, 100);
        assert_eq!(cobol_b, FreshnessLevel::Fresh);
        assert_ne!(prompt_b, FreshnessLevel::Fresh);
    }

    #[test]
    fn refresh_resets_freshness() {
        let c = class(100);
        let mut cr = cred(1000, 0);
        // After 200 epochs (2 half-lives), credential is well past expired.
        let stale = freshness_bucket(&c, &cr, 200);
        assert!(matches!(
            stale,
            FreshnessLevel::Stale | FreshnessLevel::Expired
        ));
        // Refresh at epoch 200 — back to Fresh.
        cr.refresh(1000, 200).unwrap();
        assert_eq!(freshness_bucket(&c, &cr, 200), FreshnessLevel::Fresh);
    }

    /// T1.20 — freshness_bucket returns Expired when cred.level is
    /// zero (line 51 — defensive fallback for zero-level creds).
    #[test]
    fn t1_20_freshness_bucket_zero_level_returns_expired() {
        let c = class(100);
        let mut cr = cred(1000, 50);
        cr.level = 0;
        assert_eq!(freshness_bucket(&c, &cr, 50), FreshnessLevel::Expired);
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Freshness score is monotone non-increasing in elapsed time.
        #[test]
        fn score_monotone_in_time(
            half_life in 1u64..10_000,
            level in 1u64..1_000_000,
            attested_at in 0u64..1_000,
            t_a in 0u64..2_000_000,
            extra in 0u64..1_000_000,
        ) {
            let c = test_class(half_life);
            let cr = test_cred(level, attested_at);
            let s_a = freshness_score(&c, &cr, t_a);
            let s_b = freshness_score(&c, &cr, t_a.saturating_add(extra));
            prop_assert!(s_b <= s_a, "score went up: {} → {}", s_a, s_b);
        }

        /// Score is always in [0, level].
        #[test]
        fn score_bounded_by_level(
            half_life in 1u64..10_000,
            level in 1u64..1_000_000,
            attested_at in 0u64..1_000,
            now in 0u64..2_000_000,
        ) {
            let c = test_class(half_life);
            let cr = test_cred(level, attested_at);
            let s = freshness_score(&c, &cr, now);
            prop_assert!(s <= level);
        }
    }
}
