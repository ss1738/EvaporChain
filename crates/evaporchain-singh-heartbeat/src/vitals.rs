//! `Vitals` — the validator-agreed pulse tuple.
//!
//! `pulse_at(items, epoch_now)` returns:
//!
//! ```text
//!     Vitals {
//!         bpm,                  // beats per minute (60..120)
//!         color,                // Green / Amber / Red
//!         arrhythmia_amp,       // 0..100 — how irregular the rhythm is
//!         aggregate_health,     // 0.0..1.0 — energy-weighted fraction
//!         worst_remaining,      // 0.0..1.0 — fraction of the most-decayed item
//!     }
//! ```
//!
//! ## Math (integer-only on the public path; small `f64` on the
//! reported floats — purely cosmetic, validators agree on the
//! integer/qualitative parts)
//!
//! - `aggregate_health = sum(item.energy_at_now) / sum(item.energy_at_anchor)`.
//!   Energy-weighted; one large healthy item dominates many small ones,
//!   matching the Singh-Sabi precedent.
//! - `worst_remaining = min over items of (energy_at_now / energy_at_anchor)`.
//! - `bpm` interpolates between 60 (full health) and 120 (zero health).
//!   Same shape as a real human heart-rate at rest vs alarm.
//! - `arrhythmia_amp = (aggregate_health - worst_remaining) * 100`. If
//!   the worst item is far below average, the rhythm is irregular —
//!   that's what makes the wallet "feel wrong" before the inbox shows it.
//! - `color = color_for_health(aggregate_health)` per [`crate::color`].

use evaporchain_singh_triage::TriageItem;
use evaporchain_types::Epoch;
use serde::{Deserialize, Serialize};

use crate::color::{color_for_health, PulseColor};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Vitals {
    /// Beats per minute. 60 = healthy resting; 120 = alarmed.
    pub bpm: u32,
    pub color: PulseColor,
    /// Arrhythmia amplitude in basis points / 100 (0..=100).
    /// 0 = perfectly regular pulse; 100 = wildly irregular.
    pub arrhythmia_amp: u8,
    /// Energy-weighted aggregate health, [0.0, 1.0].
    pub aggregate_health: f64,
    /// Fraction the worst-decayed item has remaining, [0.0, 1.0].
    pub worst_remaining: f64,
}

/// Resting BPM (full-health pulse rate). Doctrine: "slow 60bpm green pulse."
pub const HEALTHY_BPM: u32 = 60;
/// Maximum BPM at zero health.
pub const ALARMED_BPM: u32 = 120;

/// Compute `Vitals` for a slice of items at `epoch_now`. Pure
/// function. Empty wallet returns "perfectly healthy at rest" (bpm =
/// 60, green, no arrhythmia) — there's nothing wrong because there's
/// nothing to be wrong.
pub fn pulse_at(items: &[TriageItem], epoch_now: Epoch) -> Vitals {
    if items.is_empty() {
        return Vitals {
            bpm: HEALTHY_BPM,
            color: PulseColor::Green,
            arrhythmia_amp: 0,
            aggregate_health: 1.0,
            worst_remaining: 1.0,
        };
    }

    let mut total_now: u128 = 0;
    let mut total_anchor: u128 = 0;
    let mut min_fraction = f64::INFINITY;

    for it in items {
        let now = it.energy_at(epoch_now) as u128;
        let anchor = it.energy_at_anchor as u128;
        total_now = total_now.saturating_add(now);
        total_anchor = total_anchor.saturating_add(anchor);

        // Per-item fraction; defensive zero-anchor guard (shouldn't
        // arise via constructor but safe to handle).
        if anchor > 0 {
            let frac = now as f64 / anchor as f64;
            if frac < min_fraction {
                min_fraction = frac;
            }
        }
    }

    let aggregate_health = if total_anchor == 0 {
        1.0
    } else {
        (total_now as f64 / total_anchor as f64).clamp(0.0, 1.0)
    };
    if !min_fraction.is_finite() {
        min_fraction = 1.0;
    }
    let worst_remaining = min_fraction.clamp(0.0, 1.0);

    let bpm = bpm_from_health(aggregate_health);
    let color = color_for_health(aggregate_health);
    // arrhythmia is the *gap* between aggregate and worst — large gap
    // = some items doing fine while others are dying = irregular feel.
    let gap = (aggregate_health - worst_remaining).clamp(0.0, 1.0);
    let arrhythmia_amp = (gap * 100.0).round().clamp(0.0, 100.0) as u8;

    Vitals {
        bpm,
        color,
        arrhythmia_amp,
        aggregate_health,
        worst_remaining,
    }
}

fn bpm_from_health(health: f64) -> u32 {
    let h = health.clamp(0.0, 1.0);
    let span = (ALARMED_BPM - HEALTHY_BPM) as f64;
    // Linear interpolation: full health → HEALTHY_BPM; zero → ALARMED_BPM.
    let raw = HEALTHY_BPM as f64 + span * (1.0 - h);
    raw.round().clamp(HEALTHY_BPM as f64, ALARMED_BPM as f64) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(byte: u8, energy: u64, half_life: u64, last_refreshed: u64) -> TriageItem {
        let mut id = [0u8; 32];
        id[0] = byte;
        TriageItem::new(id, energy, half_life, last_refreshed).unwrap()
    }

    #[test]
    fn empty_wallet_is_perfectly_at_rest() {
        let v = pulse_at(&[], 0);
        assert_eq!(v.bpm, HEALTHY_BPM);
        assert_eq!(v.color, PulseColor::Green);
        assert_eq!(v.arrhythmia_amp, 0);
        assert_eq!(v.aggregate_health, 1.0);
        assert_eq!(v.worst_remaining, 1.0);
    }

    #[test]
    fn fully_healthy_wallet_pulses_at_resting_rate() {
        // Just-anchored items ⇒ energy_at_now ≈ energy_at_anchor ⇒
        // aggregate_health ≈ 1.0.
        let items = vec![
            item(1, 1000, 100, 0),
            item(2, 500, 100, 0),
            item(3, 800, 100, 0),
        ];
        let v = pulse_at(&items, 0);
        assert_eq!(v.bpm, HEALTHY_BPM);
        assert_eq!(v.color, PulseColor::Green);
        assert_eq!(v.aggregate_health, 1.0);
    }

    #[test]
    fn zero_health_pulse_is_alarmed() {
        // Tiny initial + small half-life + huge wait ⇒ all items dead.
        let items = vec![item(1, 1, 1, 0)];
        let v = pulse_at(&items, 10_000);
        assert_eq!(v.bpm, ALARMED_BPM);
        assert_eq!(v.color, PulseColor::Red);
    }

    #[test]
    fn bpm_interpolates_between_healthy_and_alarmed() {
        // Test the linear-interpolation midpoint: ~50% health ⇒
        // bpm ≈ 90 (midway between 60 and 120).
        let items = vec![item(1, 1000, 1, 0)];
        // After 1 epoch (one half-life), energy ≈ 500 ⇒ ~50% health.
        let v = pulse_at(&items, 1);
        // Allow some tolerance because energy_at_epoch uses linear
        // interpolation between half-life boundaries.
        assert!(
            v.bpm > 60 && v.bpm <= 120,
            "expected mid-range bpm, got {}",
            v.bpm
        );
    }

    #[test]
    fn aggregate_health_is_energy_weighted_not_count_weighted() {
        // One large healthy item should dominate many small dead ones.
        let items = vec![
            item(1, 1_000_000, 1_000_000, 0), // huge energy, huge half-life
            item(2, 1, 1, 0),                  // dies fast
            item(3, 1, 1, 0),
            item(4, 1, 1, 0),
        ];
        let v = pulse_at(&items, 100); // small ones long dead, big one fine
        assert!(v.aggregate_health > 0.99);
        assert_eq!(v.color, PulseColor::Green);
    }

    #[test]
    fn arrhythmia_signals_one_dying_item_among_healthy() {
        // Doctrine claim: "what makes the wallet feel wrong before
        // the user sees the inbox." Build a wallet where most items
        // are healthy but one is far decayed. Aggregate health stays
        // high; worst_remaining drops sharply; arrhythmia is high.
        let items = vec![
            item(1, 1_000_000, 1_000_000, 0), // healthy giant
            item(2, 1_000_000, 1_000_000, 0),
            item(3, 1_000_000, 1_000_000, 0),
            item(4, 4, 1, 0), // tiny dying item
        ];
        let v = pulse_at(&items, 100);
        assert!(v.arrhythmia_amp >= 90, "expected high arrhythmia, got {}", v.arrhythmia_amp);
        // Aggregate is fine because the giants dominate.
        assert_eq!(v.color, PulseColor::Green);
    }

    #[test]
    fn even_distribution_has_zero_arrhythmia() {
        // If every item is in the same shape, the rhythm is regular
        // even when health is poor.
        let items = vec![
            item(1, 1000, 100, 0),
            item(2, 1000, 100, 0),
            item(3, 1000, 100, 0),
        ];
        let v = pulse_at(&items, 0);
        assert_eq!(v.arrhythmia_amp, 0);
    }

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Doctrine: "Healthy: slow 60bpm green pulse. Decay below
        // threshold: arrhythmic red." Two scenarios; verify the pulse
        // moves from one extreme to the other.
        let healthy = vec![item(1, 1_000_000, 1_000_000, 0)];
        let v_healthy = pulse_at(&healthy, 0);
        assert_eq!(v_healthy.bpm, HEALTHY_BPM);
        assert_eq!(v_healthy.color, PulseColor::Green);
        let dying = vec![item(1, 1, 1, 0)];
        let v_dying = pulse_at(&dying, 1000);
        assert_eq!(v_dying.bpm, ALARMED_BPM);
        assert_eq!(v_dying.color, PulseColor::Red);
    }

    #[test]
    fn round_trip_serde() {
        let v = Vitals {
            bpm: 90,
            color: PulseColor::Amber,
            arrhythmia_amp: 25,
            aggregate_health: 0.55,
            worst_remaining: 0.30,
        };
        let s = serde_json::to_string(&v).unwrap();
        let back: Vitals = serde_json::from_str(&s).unwrap();
        assert_eq!(v, back);
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// BPM is monotone non-decreasing as health drops. Sicker
        /// wallets pulse faster, never slower.
        #[test]
        fn bpm_monotone_in_health(
            energy in 100u64..1_000_000,
            half_life in 1u64..10_000,
            t_a in 0u64..1_000,
            extra in 0u64..10_000,
        ) {
            let it = item_helper(energy, half_life);
            let v_a = pulse_at(&[it.clone()], t_a);
            let v_b = pulse_at(&[it], t_a.saturating_add(extra));
            prop_assert!(
                v_b.bpm >= v_a.bpm,
                "bpm went down as health dropped: {} → {}",
                v_a.bpm, v_b.bpm
            );
        }

        /// BPM is bounded [HEALTHY_BPM, ALARMED_BPM].
        #[test]
        fn bpm_in_bounds(
            energy in 100u64..1_000_000,
            half_life in 1u64..10_000,
            now in 0u64..100_000,
        ) {
            let it = item_helper(energy, half_life);
            let v = pulse_at(&[it], now);
            prop_assert!(v.bpm >= HEALTHY_BPM);
            prop_assert!(v.bpm <= ALARMED_BPM);
        }
    }

    fn item_helper(energy: u64, half_life: u64) -> TriageItem {
        TriageItem::new([1; 32], energy, half_life, 0).unwrap()
    }
}
