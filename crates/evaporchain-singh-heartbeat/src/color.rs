//! `PulseColor` — qualitative ambient signal.
//!
//! The wallet maps these to brand-specific hex codes; the chain only
//! agrees on the qualitative state. Three thresholds enforce a clean
//! traffic-light intuition:
//!
//! - **Green** — aggregate health ≥ 0.75; "everything is fine"
//! - **Amber** — 0.40 ≤ health < 0.75; "pay attention"
//! - **Red** — health < 0.40; "something is dying"
//!
//! Health is a fraction in [0, 1]; the caller computes it elsewhere
//! (typically via [`crate::vitals::pulse_at`]).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PulseColor {
    /// Wallet is healthy. Aggregate fraction ≥ 0.75.
    Green,
    /// Items are decaying — pay attention. 0.40 ≤ fraction < 0.75.
    Amber,
    /// Wallet is failing. Fraction < 0.40.
    Red,
}

/// Threshold below which the pulse turns Amber.
pub const AMBER_THRESHOLD_BP: u32 = 75; // 0.75 in basis points / 100
/// Threshold below which the pulse turns Red.
pub const RED_THRESHOLD_BP: u32 = 40; // 0.40

/// Map a health fraction in [0.0, 1.0] to its qualitative `PulseColor`.
///
/// Pure function; deterministic; integer-only conversion via the
/// basis-point threshold consts (so validators agree without floating-
/// point divergence even if the caller did the division on `f64`).
/// Out-of-range health values clamp to the nearest valid bucket.
pub fn color_for_health(health: f64) -> PulseColor {
    let bp = (health * 100.0).clamp(0.0, 100.0) as u32;
    if bp >= AMBER_THRESHOLD_BP {
        PulseColor::Green
    } else if bp >= RED_THRESHOLD_BP {
        PulseColor::Amber
    } else {
        PulseColor::Red
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_health_is_green() {
        assert_eq!(color_for_health(1.0), PulseColor::Green);
        assert_eq!(color_for_health(0.999), PulseColor::Green);
        assert_eq!(color_for_health(0.75), PulseColor::Green);
    }

    #[test]
    fn middle_health_is_amber() {
        assert_eq!(color_for_health(0.74), PulseColor::Amber);
        assert_eq!(color_for_health(0.50), PulseColor::Amber);
        assert_eq!(color_for_health(0.40), PulseColor::Amber);
    }

    #[test]
    fn low_health_is_red() {
        assert_eq!(color_for_health(0.39), PulseColor::Red);
        assert_eq!(color_for_health(0.10), PulseColor::Red);
        assert_eq!(color_for_health(0.0), PulseColor::Red);
    }

    #[test]
    fn out_of_range_health_clamps() {
        // Negative or > 1.0: defensive clamp.
        assert_eq!(color_for_health(-0.5), PulseColor::Red);
        assert_eq!(color_for_health(2.0), PulseColor::Green);
    }

    #[test]
    fn round_trip_serde() {
        for c in [PulseColor::Green, PulseColor::Amber, PulseColor::Red] {
            let s = serde_json::to_string(&c).unwrap();
            let back: PulseColor = serde_json::from_str(&s).unwrap();
            assert_eq!(c, back);
        }
    }
}
