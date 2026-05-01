//! Fee market analytics — gas price trends, percentiles, and predictions.
//!
//! Tracks historical base fees to help users:
//! - See if current fees are high/low relative to history
//! - Find optimal timing for non-urgent transactions
//! - Set fee alerts for target gas prices
//! - Understand fee trends over time
//!
//! Data is collected from block headers and stored locally.

use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum FeeAnalyticsError {
    #[error("not enough data: need at least {needed} samples, have {have}")]
    InsufficientData { needed: usize, have: usize },
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

// ──────────────────────────── Types ──────────────────────────────────────

/// A single fee data point.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeeSample {
    pub block_number: u64,
    pub epoch: u64,
    pub base_fee: u64,
    pub gas_used: u64,
    pub tx_count: u64,
    pub timestamp: String,
}

/// Fee statistics over a window.
#[derive(Debug, Clone, Serialize)]
pub struct FeeStats {
    /// Number of samples.
    pub samples: usize,
    /// Minimum base fee seen.
    pub min: u64,
    /// Maximum base fee seen.
    pub max: u64,
    /// Average base fee.
    pub avg: u64,
    /// Median base fee.
    pub median: u64,
    /// 25th percentile.
    pub p25: u64,
    /// 75th percentile.
    pub p75: u64,
    /// 90th percentile.
    pub p90: u64,
    /// Current base fee.
    pub current: u64,
    /// Where current fee sits relative to history (0-100).
    pub current_percentile: f64,
    /// Trend direction.
    pub trend: FeeTrend,
}

/// Fee trend direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FeeTrend {
    Rising,
    Falling,
    Stable,
}

/// A fee alert configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeeAlert {
    /// Alert name.
    pub name: String,
    /// Target base fee (alert when fee drops below this).
    pub target_fee: u64,
    /// Whether alert is enabled.
    pub enabled: bool,
    /// Whether alert has fired.
    pub fired: bool,
}

/// Fee timing recommendation.
#[derive(Debug, Clone, Serialize)]
pub struct TimingAdvice {
    /// Current fee level (low/medium/high/very_high).
    pub level: String,
    /// Recommendation text.
    pub advice: String,
    /// Estimated savings if waiting (vs current fee).
    pub potential_savings_pct: f64,
}

// ──────────────────────────── FeeTracker ─────────────────────────────────

/// Tracks and analyzes fee history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeeTracker {
    /// Historical fee samples (ordered by block number).
    pub samples: Vec<FeeSample>,
    /// Maximum samples to retain.
    pub max_samples: usize,
    /// Fee alerts.
    pub alerts: Vec<FeeAlert>,
}

impl FeeTracker {
    /// Create a new tracker.
    pub fn new(max_samples: usize) -> Self {
        Self {
            samples: Vec::new(),
            max_samples,
            alerts: Vec::new(),
        }
    }

    /// Create with default capacity (1000 samples ≈ ~1000 blocks).
    pub fn default_capacity() -> Self {
        Self::new(1000)
    }

    /// Load from a JSON file.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, FeeAnalyticsError> {
        let data = std::fs::read_to_string(path)?;
        let tracker: FeeTracker = serde_json::from_str(&data)?;
        Ok(tracker)
    }

    /// Save to a JSON file.
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), FeeAnalyticsError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Record a new fee sample.
    pub fn record(&mut self, sample: FeeSample) {
        self.samples.push(sample);
        // Trim if over capacity
        if self.samples.len() > self.max_samples {
            let excess = self.samples.len() - self.max_samples;
            self.samples.drain(..excess);
        }
    }

    /// Record from block data.
    pub fn record_block(
        &mut self,
        block_number: u64,
        epoch: u64,
        base_fee: u64,
        gas_used: u64,
        tx_count: u64,
    ) {
        self.record(FeeSample {
            block_number,
            epoch,
            base_fee,
            gas_used,
            tx_count,
            timestamp: chrono::Utc::now().to_rfc3339(),
        });
    }

    /// Get the latest base fee.
    pub fn current_fee(&self) -> Option<u64> {
        self.samples.last().map(|s| s.base_fee)
    }

    /// Compute fee statistics over all samples.
    pub fn stats(&self) -> Result<FeeStats, FeeAnalyticsError> {
        if self.samples.len() < 2 {
            return Err(FeeAnalyticsError::InsufficientData {
                needed: 2,
                have: self.samples.len(),
            });
        }

        let mut fees: Vec<u64> = self.samples.iter().map(|s| s.base_fee).collect();
        fees.sort_unstable();

        let n = fees.len();
        let sum: u64 = fees.iter().sum();
        let avg = sum / n as u64;
        let min = fees[0];
        let max = fees[n - 1];
        let median = fees[n / 2];
        let p25 = fees[n / 4];
        let p75 = fees[3 * n / 4];
        let p90 = fees[9 * n / 10];

        let current = self.samples.last().map(|s| s.base_fee).unwrap_or(0);

        // Compute percentile of current fee
        let below = fees.iter().filter(|&&f| f < current).count();
        let current_percentile = (below as f64 / n as f64) * 100.0;

        // Compute trend from last 10 samples
        let trend = self.compute_trend();

        Ok(FeeStats {
            samples: n,
            min,
            max,
            avg,
            median,
            p25,
            p75,
            p90,
            current,
            current_percentile,
            trend,
        })
    }

    /// Compute recent fee statistics (last N samples).
    pub fn recent_stats(&self, window: usize) -> Result<FeeStats, FeeAnalyticsError> {
        if self.samples.len() < window.min(2) {
            return Err(FeeAnalyticsError::InsufficientData {
                needed: window.min(2),
                have: self.samples.len(),
            });
        }

        let start = self.samples.len().saturating_sub(window);
        let recent: Vec<u64> = self.samples[start..].iter().map(|s| s.base_fee).collect();

        let mut sorted = recent.clone();
        sorted.sort_unstable();
        let n = sorted.len();
        let sum: u64 = sorted.iter().sum();

        let current = *sorted.last().unwrap_or(&0);
        let below = sorted.iter().filter(|&&f| f < current).count();

        Ok(FeeStats {
            samples: n,
            min: sorted[0],
            max: sorted[n - 1],
            avg: sum / n as u64,
            median: sorted[n / 2],
            p25: sorted[n / 4],
            p75: sorted[3 * n / 4],
            p90: sorted[9 * n / 10],
            current,
            current_percentile: (below as f64 / n as f64) * 100.0,
            trend: self.compute_trend(),
        })
    }

    /// Get timing advice based on current fee vs history.
    pub fn timing_advice(&self) -> Result<TimingAdvice, FeeAnalyticsError> {
        let stats = self.stats()?;

        let (level, advice, savings) = if stats.current <= stats.p25 {
            ("low", "Fees are low — great time to transact!", 0.0)
        } else if stats.current <= stats.median {
            (
                "medium",
                "Fees are moderate — reasonable to transact now.",
                ((stats.current - stats.p25) as f64 / stats.current as f64) * 100.0,
            )
        } else if stats.current <= stats.p90 {
            (
                "high",
                "Fees are elevated — consider waiting for lower rates.",
                ((stats.current - stats.median) as f64 / stats.current as f64) * 100.0,
            )
        } else {
            (
                "very_high",
                "Fees are very high — strongly recommend waiting unless urgent.",
                ((stats.current - stats.median) as f64 / stats.current as f64) * 100.0,
            )
        };

        Ok(TimingAdvice {
            level: level.to_string(),
            advice: advice.to_string(),
            potential_savings_pct: (savings * 10.0).round() / 10.0,
        })
    }

    // ── Alerts ──────────────────────────────────────────────────────────

    /// Add a fee alert.
    pub fn add_alert(&mut self, name: &str, target_fee: u64) {
        self.alerts.retain(|a| a.name != name);
        self.alerts.push(FeeAlert {
            name: name.to_string(),
            target_fee,
            enabled: true,
            fired: false,
        });
    }

    /// Remove a fee alert.
    pub fn remove_alert(&mut self, name: &str) -> bool {
        let before = self.alerts.len();
        self.alerts.retain(|a| a.name != name);
        self.alerts.len() < before
    }

    /// Check alerts against current fee, return names of triggered alerts.
    pub fn check_alerts(&mut self) -> Vec<String> {
        let current = match self.current_fee() {
            Some(f) => f,
            None => return Vec::new(),
        };

        let mut triggered = Vec::new();
        for alert in &mut self.alerts {
            if alert.enabled && !alert.fired && current <= alert.target_fee {
                alert.fired = true;
                triggered.push(alert.name.clone());
            }
        }
        triggered
    }

    /// Reset a fired alert.
    pub fn reset_alert(&mut self, name: &str) {
        if let Some(alert) = self.alerts.iter_mut().find(|a| a.name == name) {
            alert.fired = false;
        }
    }

    /// List alerts.
    pub fn list_alerts(&self) -> &[FeeAlert] {
        &self.alerts
    }

    // ── Internal ────────────────────────────────────────────────────────

    fn compute_trend(&self) -> FeeTrend {
        let n = self.samples.len();
        if n < 5 {
            return FeeTrend::Stable;
        }

        let window = n.min(10);
        let recent = &self.samples[n - window..];
        let first_half_avg: u64 =
            recent[..window / 2].iter().map(|s| s.base_fee).sum::<u64>() / (window / 2) as u64;
        let second_half_avg: u64 = recent[window / 2..].iter().map(|s| s.base_fee).sum::<u64>()
            / (window - window / 2) as u64;

        let change_pct = if first_half_avg > 0 {
            ((second_half_avg as f64 - first_half_avg as f64) / first_half_avg as f64) * 100.0
        } else {
            0.0
        };

        if change_pct > 5.0 {
            FeeTrend::Rising
        } else if change_pct < -5.0 {
            FeeTrend::Falling
        } else {
            FeeTrend::Stable
        }
    }

    /// Number of recorded samples.
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Whether tracker has any data.
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
}

impl Default for FeeTracker {
    fn default() -> Self {
        Self::default_capacity()
    }
}

/// Default path for fee tracker data.
pub fn default_fee_path() -> std::path::PathBuf {
    crate::config::default_data_dir().join("fee_history.json")
}

// ──────────────────────────── Tests ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tracker() -> FeeTracker {
        let mut tracker = FeeTracker::new(100);
        // Add 20 samples with varying fees
        for i in 0..20 {
            tracker.record_block(i, i / 10, 100 + (i % 5) * 20, 50_000, 10);
        }
        tracker
    }

    fn make_trending_tracker(direction: &str) -> FeeTracker {
        let mut tracker = FeeTracker::new(100);
        for i in 0..20 {
            let fee = match direction {
                "rising" => 100 + i * 10,
                "falling" => 300 - i * 10,
                _ => 150,
            };
            tracker.record_block(i, i / 10, fee, 50_000, 10);
        }
        tracker
    }

    #[test]
    fn test_record_sample() {
        let mut tracker = FeeTracker::new(10);
        tracker.record_block(1, 0, 100, 50_000, 5);
        assert_eq!(tracker.len(), 1);
        assert_eq!(tracker.current_fee(), Some(100));
    }

    #[test]
    fn test_capacity_limit() {
        let mut tracker = FeeTracker::new(5);
        for i in 0..10 {
            tracker.record_block(i, 0, 100 + i, 50_000, 5);
        }
        assert_eq!(tracker.len(), 5);
        // Oldest should be trimmed
        assert_eq!(tracker.samples[0].block_number, 5);
    }

    #[test]
    fn test_stats_basic() {
        let tracker = make_tracker();
        let stats = tracker.stats().unwrap();
        assert_eq!(stats.samples, 20);
        assert!(stats.min <= stats.median);
        assert!(stats.median <= stats.max);
        assert!(stats.p25 <= stats.median);
        assert!(stats.median <= stats.p75);
        assert!(stats.p75 <= stats.p90);
    }

    #[test]
    fn test_stats_insufficient_data() {
        let tracker = FeeTracker::new(100);
        let err = tracker.stats().unwrap_err();
        assert!(matches!(err, FeeAnalyticsError::InsufficientData { .. }));
    }

    #[test]
    fn test_stats_single_sample() {
        let mut tracker = FeeTracker::new(100);
        tracker.record_block(0, 0, 100, 50_000, 5);
        let err = tracker.stats().unwrap_err();
        assert!(matches!(err, FeeAnalyticsError::InsufficientData { .. }));
    }

    #[test]
    fn test_trend_rising() {
        let tracker = make_trending_tracker("rising");
        let stats = tracker.stats().unwrap();
        assert_eq!(stats.trend, FeeTrend::Rising);
    }

    #[test]
    fn test_trend_falling() {
        let tracker = make_trending_tracker("falling");
        let stats = tracker.stats().unwrap();
        assert_eq!(stats.trend, FeeTrend::Falling);
    }

    #[test]
    fn test_trend_stable() {
        let tracker = make_trending_tracker("stable");
        let stats = tracker.stats().unwrap();
        assert_eq!(stats.trend, FeeTrend::Stable);
    }

    #[test]
    fn test_recent_stats() {
        let tracker = make_tracker();
        let stats = tracker.recent_stats(5).unwrap();
        assert_eq!(stats.samples, 5);
    }

    #[test]
    fn test_timing_advice_low() {
        let mut tracker = FeeTracker::new(100);
        // Many high fees, then a low one
        for i in 0..19 {
            tracker.record_block(i, 0, 200, 50_000, 5);
        }
        tracker.record_block(19, 0, 50, 50_000, 5); // current is low
        let advice = tracker.timing_advice().unwrap();
        assert_eq!(advice.level, "low");
    }

    #[test]
    fn test_timing_advice_high() {
        let mut tracker = FeeTracker::new(100);
        // Many low fees, then a high one
        for i in 0..19 {
            tracker.record_block(i, 0, 50, 50_000, 5);
        }
        tracker.record_block(19, 0, 500, 50_000, 5); // current is high
        let advice = tracker.timing_advice().unwrap();
        assert!(advice.level == "high" || advice.level == "very_high");
    }

    #[test]
    fn test_add_alert() {
        let mut tracker = FeeTracker::new(100);
        tracker.add_alert("low_fee", 80);
        assert_eq!(tracker.list_alerts().len(), 1);
        assert_eq!(tracker.list_alerts()[0].target_fee, 80);
    }

    #[test]
    fn test_add_alert_replaces() {
        let mut tracker = FeeTracker::new(100);
        tracker.add_alert("low_fee", 80);
        tracker.add_alert("low_fee", 90);
        assert_eq!(tracker.list_alerts().len(), 1);
        assert_eq!(tracker.list_alerts()[0].target_fee, 90);
    }

    #[test]
    fn test_remove_alert() {
        let mut tracker = FeeTracker::new(100);
        tracker.add_alert("low_fee", 80);
        assert!(tracker.remove_alert("low_fee"));
        assert!(tracker.list_alerts().is_empty());
    }

    #[test]
    fn test_remove_alert_not_found() {
        let mut tracker = FeeTracker::new(100);
        assert!(!tracker.remove_alert("nope"));
    }

    #[test]
    fn test_check_alerts_triggered() {
        let mut tracker = FeeTracker::new(100);
        tracker.add_alert("cheap", 100);
        tracker.record_block(0, 0, 80, 50_000, 5); // fee below target
        let triggered = tracker.check_alerts();
        assert_eq!(triggered, vec!["cheap"]);
        // Should not trigger again
        let triggered = tracker.check_alerts();
        assert!(triggered.is_empty());
    }

    #[test]
    fn test_check_alerts_not_triggered() {
        let mut tracker = FeeTracker::new(100);
        tracker.add_alert("cheap", 50);
        tracker.record_block(0, 0, 100, 50_000, 5); // fee above target
        let triggered = tracker.check_alerts();
        assert!(triggered.is_empty());
    }

    #[test]
    fn test_reset_alert() {
        let mut tracker = FeeTracker::new(100);
        tracker.add_alert("cheap", 100);
        tracker.record_block(0, 0, 80, 50_000, 5);
        tracker.check_alerts();
        assert!(tracker.list_alerts()[0].fired);
        tracker.reset_alert("cheap");
        assert!(!tracker.list_alerts()[0].fired);
    }

    #[test]
    fn test_current_fee_empty() {
        let tracker = FeeTracker::new(100);
        assert!(tracker.current_fee().is_none());
    }

    #[test]
    fn test_is_empty() {
        let tracker = FeeTracker::new(100);
        assert!(tracker.is_empty());
        let tracker = make_tracker();
        assert!(!tracker.is_empty());
    }

    #[test]
    fn test_fee_stats_serializable() {
        let tracker = make_tracker();
        let stats = tracker.stats().unwrap();
        let json = serde_json::to_string(&stats).unwrap();
        assert!(json.contains("\"samples\":20"));
        assert!(json.contains("\"trend\""));
    }

    #[test]
    fn test_timing_advice_serializable() {
        let tracker = make_tracker();
        let advice = tracker.timing_advice().unwrap();
        let json = serde_json::to_string(&advice).unwrap();
        assert!(json.contains("\"level\""));
        assert!(json.contains("\"advice\""));
    }

    #[test]
    fn test_json_roundtrip() {
        let mut tracker = make_tracker();
        tracker.add_alert("test", 50);

        let json = serde_json::to_string_pretty(&tracker).unwrap();
        let loaded: FeeTracker = serde_json::from_str(&json).unwrap();

        assert_eq!(loaded.len(), 20);
        assert_eq!(loaded.list_alerts().len(), 1);
    }

    #[test]
    fn test_file_save_and_load() {
        let dir = std::env::temp_dir().join("evaporchain_fee_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fees.json");

        let tracker = make_tracker();
        tracker.save(&path).unwrap();

        let loaded = FeeTracker::load(&path).unwrap();
        assert_eq!(loaded.len(), 20);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_fee_alert_serializable() {
        let alert = FeeAlert {
            name: "low".into(),
            target_fee: 80,
            enabled: true,
            fired: false,
        };
        let json = serde_json::to_string(&alert).unwrap();
        assert!(json.contains("\"target_fee\":80"));
    }

    #[test]
    fn test_default_capacity() {
        let tracker = FeeTracker::default_capacity();
        assert_eq!(tracker.max_samples, 1000);
    }
}
