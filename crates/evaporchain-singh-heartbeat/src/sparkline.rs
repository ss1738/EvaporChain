//! Sparkline — 24-tick history series for the watch complication.
//!
//! Per A5.4: "Apple Watch complication shows sparkline of wallet's
//! heartbeat over 24h." The chain doesn't draw pixels; it returns
//! the 24 sample points the wallet's UI renders. The wallet picks
//! the time-base (typically 1 sample per hour over a 24-hour window,
//! but the function takes an explicit step so a watch face wanting
//! 24 samples over 6 hours can have it).

use evaporchain_singh_triage::TriageItem;
use evaporchain_types::Epoch;
use serde::{Deserialize, Serialize};

use crate::vitals::{pulse_at, Vitals};

/// One point in the sparkline history.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SparklinePoint {
    pub epoch: Epoch,
    pub vitals: Vitals,
}

/// Compute a 24-tick history of `Vitals` ending at `epoch_now`,
/// stepping back by `step_epochs` between samples. Returns 24 points
/// in chronological order (oldest first, newest last).
///
/// If `epoch_now < 23 * step_epochs`, the earliest samples are at
/// `epoch = 0` (clipped, not negative).
pub fn sparkline_24h(
    items: &[TriageItem],
    epoch_now: Epoch,
    step_epochs: u64,
) -> Vec<SparklinePoint> {
    const TICKS: u64 = 24;
    let step = step_epochs.max(1);
    let mut out = Vec::with_capacity(TICKS as usize);
    for i in (0..TICKS).rev() {
        let offset = i.saturating_mul(step);
        let epoch = epoch_now.saturating_sub(offset);
        out.push(SparklinePoint {
            epoch,
            vitals: pulse_at(items, epoch),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::PulseColor;

    fn item(byte: u8, energy: u64, half_life: u64, last_refreshed: u64) -> TriageItem {
        let mut id = [0u8; 32];
        id[0] = byte;
        TriageItem::new(id, energy, half_life, last_refreshed).unwrap()
    }

    #[test]
    fn sparkline_returns_24_points() {
        let items = vec![item(1, 1000, 100, 0)];
        let pts = sparkline_24h(&items, 100, 1);
        assert_eq!(pts.len(), 24);
    }

    #[test]
    fn sparkline_is_chronological() {
        let items = vec![item(1, 1000, 100, 0)];
        let pts = sparkline_24h(&items, 100, 5);
        // First epoch is the oldest; last is `epoch_now`.
        assert!(pts.first().unwrap().epoch <= pts.last().unwrap().epoch);
        assert_eq!(pts.last().unwrap().epoch, 100);
    }

    #[test]
    fn sparkline_clips_at_zero_for_short_history() {
        // Wallet that's only 5 epochs old: requesting a 24h sparkline
        // should clip the earliest samples at epoch=0, not crash.
        let items = vec![item(1, 1000, 100, 0)];
        let pts = sparkline_24h(&items, 5, 10);
        assert_eq!(pts.len(), 24);
        assert_eq!(pts.first().unwrap().epoch, 0);
        assert_eq!(pts.last().unwrap().epoch, 5);
    }

    #[test]
    fn sparkline_shows_progression_for_decaying_wallet() {
        // Doctrine: the sparkline should show health *deteriorating*
        // over time. Anchor the item so it's fresh at the sparkline's
        // first tick and dead by the last. With step=1 and 24 ticks,
        // sparkline spans [epoch_now - 23, epoch_now]. Set last
        // refresh to the start of the window.
        let items = vec![item(1, 4, 2, 77)];
        let pts = sparkline_24h(&items, 100, 1);
        let earliest = pts.first().unwrap();
        let latest = pts.last().unwrap();
        assert_eq!(earliest.vitals.color, PulseColor::Green);
        assert_eq!(latest.vitals.color, PulseColor::Red);
        // BPM increases over the series.
        assert!(latest.vitals.bpm >= earliest.vitals.bpm);
    }

    #[test]
    fn step_zero_is_treated_as_one() {
        // Defensive guard: zero step would produce 24 identical points.
        // We coerce to step=1 so the call is at least monotonic in epoch.
        let items = vec![item(1, 1000, 100, 0)];
        let pts = sparkline_24h(&items, 100, 0);
        assert_eq!(pts.len(), 24);
        for w in pts.windows(2) {
            assert!(w[0].epoch <= w[1].epoch);
        }
    }

    #[test]
    fn empty_wallet_sparkline_is_perfectly_healthy() {
        let pts = sparkline_24h(&[], 100, 1);
        assert_eq!(pts.len(), 24);
        for p in &pts {
            assert_eq!(p.vitals.color, PulseColor::Green);
            assert_eq!(p.vitals.bpm, 60);
        }
    }

    #[test]
    fn round_trip_serde() {
        let items = vec![item(1, 1000, 100, 0)];
        let pts = sparkline_24h(&items, 100, 1);
        let s = serde_json::to_string(&pts).unwrap();
        let back: Vec<SparklinePoint> = serde_json::from_str(&s).unwrap();
        assert_eq!(pts, back);
    }
}
