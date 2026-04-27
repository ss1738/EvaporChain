use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum FeeOptimizerError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Insufficient data: {0}")]
    InsufficientData(String),
    #[error("Invalid parameter: {0}")]
    InvalidParameter(String),
}

type Result<T> = std::result::Result<T, FeeOptimizerError>;

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum FeeSpeed {
    Slow,
    Standard,
    Fast,
    Instant,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MarketCondition {
    Low,
    Normal,
    High,
    Congested,
}

// ---------------------------------------------------------------------------
// Structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeeEstimate {
    pub speed: FeeSpeed,
    pub gas_price: u64,
    pub estimated_time_secs: u64,
    pub confidence_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeeHistoryEntry {
    pub timestamp: String,
    pub gas_price: u64,
    pub block_utilization_pct: f64,
    pub tx_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeeMarketAnalysis {
    pub current_condition: MarketCondition,
    pub avg_gas_24h: f64,
    pub median_gas_24h: f64,
    pub min_gas_24h: u64,
    pub max_gas_24h: u64,
    pub trend: f64,
    pub best_hour: u32,
    pub worst_hour: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmissionWindow {
    pub start_hour: u32,
    pub end_hour: u32,
    pub avg_savings_pct: f64,
    pub recommended: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeeOptimizerStats {
    pub data_points: usize,
    pub avg_gas_price: f64,
    pub median_gas_price: f64,
    pub current_condition: MarketCondition,
    pub savings_opportunities: usize,
    pub total_estimated_savings: u64,
}

// ---------------------------------------------------------------------------
// Main Store
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FeeOptimizer {
    pub history: Vec<FeeHistoryEntry>,
    pub max_history: usize,
}

impl FeeOptimizer {
    /// Create a new FeeOptimizer with default settings.
    pub fn new() -> Self {
        Self {
            history: Vec::new(),
            max_history: 10_000,
        }
    }

    // -- persistence --------------------------------------------------------

    pub fn load(path: &Path) -> Result<Self> {
        let data = std::fs::read_to_string(path)?;
        let store: Self = serde_json::from_str(&data)?;
        Ok(store)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let data = serde_json::to_string_pretty(self)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, data)?;
        Ok(())
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }

    // -- recording ----------------------------------------------------------

    /// Append a fee history entry. Prunes oldest entries when max_history is
    /// exceeded.
    pub fn record_fee(&mut self, entry: FeeHistoryEntry) {
        self.history.push(entry);
        while self.history.len() > self.max_history {
            self.history.remove(0);
        }
    }

    // -- estimation ---------------------------------------------------------

    /// Return fee estimates for all four speed tiers based on recent gas price
    /// percentiles.
    pub fn estimate_fees(&self) -> Vec<FeeEstimate> {
        let slow_gas = self.percentile(25.0);
        let standard_gas = self.percentile(50.0);
        let fast_gas = self.percentile(75.0);
        let instant_gas = self.percentile(95.0);

        vec![
            FeeEstimate {
                speed: FeeSpeed::Slow,
                gas_price: slow_gas,
                estimated_time_secs: 600,
                confidence_pct: 70.0,
            },
            FeeEstimate {
                speed: FeeSpeed::Standard,
                gas_price: standard_gas,
                estimated_time_secs: 180,
                confidence_pct: 85.0,
            },
            FeeEstimate {
                speed: FeeSpeed::Fast,
                gas_price: fast_gas,
                estimated_time_secs: 60,
                confidence_pct: 95.0,
            },
            FeeEstimate {
                speed: FeeSpeed::Instant,
                gas_price: instant_gas,
                estimated_time_secs: 15,
                confidence_pct: 99.0,
            },
        ]
    }

    // -- analysis -----------------------------------------------------------

    /// Analyse the last 24 hours of fee history.
    pub fn market_analysis(&self) -> FeeMarketAnalysis {
        if self.history.is_empty() {
            return FeeMarketAnalysis {
                current_condition: MarketCondition::Low,
                avg_gas_24h: 0.0,
                median_gas_24h: 0.0,
                min_gas_24h: 0,
                max_gas_24h: 0,
                trend: 0.0,
                best_hour: 0,
                worst_hour: 0,
            };
        }

        let prices: Vec<u64> = self.history.iter().map(|e| e.gas_price).collect();
        let avg = prices.iter().sum::<u64>() as f64 / prices.len() as f64;
        let median = {
            let mut sorted = prices.clone();
            sorted.sort();
            sorted[sorted.len() / 2]
        };
        let min_gas = *prices.iter().min().unwrap();
        let max_gas = *prices.iter().max().unwrap();

        // Trend: compare second half average to first half average.
        let mid = prices.len() / 2;
        let trend = if mid > 0 {
            let first_avg = prices[..mid].iter().sum::<u64>() as f64 / mid as f64;
            let second_avg = prices[mid..].iter().sum::<u64>() as f64
                / (prices.len() - mid) as f64;
            second_avg - first_avg
        } else {
            0.0
        };

        // Best / worst hour: bucket by simulated hour index.
        let mut hour_totals: HashMap<u32, (u64, u32)> = HashMap::new();
        for (i, entry) in self.history.iter().enumerate() {
            let hour = (i as u32) % 24;
            let e = hour_totals.entry(hour).or_insert((0, 0));
            e.0 += entry.gas_price;
            e.1 += 1;
        }
        let best_hour = hour_totals
            .iter()
            .min_by_key(|(_, (total, count))| *total / (*count).max(1) as u64)
            .map(|(h, _)| *h)
            .unwrap_or(0);
        let worst_hour = hour_totals
            .iter()
            .max_by_key(|(_, (total, count))| *total / (*count).max(1) as u64)
            .map(|(h, _)| *h)
            .unwrap_or(0);

        // Determine market condition from avg utilization.
        let avg_util: f64 = self
            .history
            .iter()
            .map(|e| e.block_utilization_pct)
            .sum::<f64>()
            / self.history.len() as f64;

        let condition = if avg_util > 90.0 {
            MarketCondition::Congested
        } else if avg_util > 70.0 {
            MarketCondition::High
        } else if avg_util > 40.0 {
            MarketCondition::Normal
        } else {
            MarketCondition::Low
        };

        FeeMarketAnalysis {
            current_condition: condition,
            avg_gas_24h: avg,
            median_gas_24h: median as f64,
            min_gas_24h: min_gas,
            max_gas_24h: max_gas,
            trend,
            best_hour,
            worst_hour,
        }
    }

    /// Interpolate gas price for a desired confirmation time (seconds).
    pub fn predict_gas(&self, target_time_secs: u64) -> u64 {
        // Map time to percentile: faster confirmation -> higher percentile.
        // Instant (15s) ~ 95th, Slow (600s) ~ 25th.
        let pct = if target_time_secs <= 15 {
            95.0
        } else if target_time_secs >= 600 {
            25.0
        } else {
            // Linear interpolation between 95 and 25 over 15..600.
            95.0 - (target_time_secs as f64 - 15.0) / (600.0 - 15.0) * (95.0 - 25.0)
        };
        self.percentile(pct)
    }

    /// Find the cheapest 4-hour windows in the day.
    pub fn optimal_windows(&self) -> Vec<SubmissionWindow> {
        if self.history.is_empty() {
            return Vec::new();
        }

        let overall_avg = self.history.iter().map(|e| e.gas_price).sum::<u64>() as f64
            / self.history.len() as f64;

        // Bucket entries by hour (0-23).
        let mut hour_prices: HashMap<u32, Vec<u64>> = HashMap::new();
        for (i, entry) in self.history.iter().enumerate() {
            let hour = (i as u32) % 24;
            hour_prices.entry(hour).or_default().push(entry.gas_price);
        }

        let hour_avgs: HashMap<u32, f64> = hour_prices
            .iter()
            .map(|(h, prices)| {
                let avg = prices.iter().sum::<u64>() as f64 / prices.len() as f64;
                (*h, avg)
            })
            .collect();

        // Slide a 4-hour window across the 24-hour day.
        let mut windows = Vec::new();
        for start in (0..24).step_by(4) {
            let end = (start + 4) % 24;
            let mut total = 0.0;
            let mut count = 0;
            for offset in 0..4 {
                let h = (start + offset) % 24;
                if let Some(&avg) = hour_avgs.get(&h) {
                    total += avg;
                    count += 1;
                }
            }
            let window_avg = if count > 0 { total / count as f64 } else { overall_avg };
            let savings_pct = if overall_avg > 0.0 {
                ((overall_avg - window_avg) / overall_avg) * 100.0
            } else {
                0.0
            };
            let recommended = savings_pct > 5.0;

            windows.push(SubmissionWindow {
                start_hour: start,
                end_hour: end,
                avg_savings_pct: savings_pct,
                recommended,
            });
        }

        windows.sort_by(|a, b| {
            b.avg_savings_pct
                .partial_cmp(&a.avg_savings_pct)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        windows
    }

    /// Returns true if the current (latest) gas price is at or below `max_gas`.
    pub fn should_submit_now(&self, max_gas: u64) -> bool {
        match self.history.last() {
            Some(entry) => entry.gas_price <= max_gas,
            None => true,
        }
    }

    /// Hours to wait for gas to drop below `max_gas`, or `None` if now is
    /// acceptable.
    pub fn wait_recommendation(&self, max_gas: u64) -> Option<u32> {
        if self.should_submit_now(max_gas) {
            return None;
        }

        // Estimate wait based on historical cheapest hour.
        let analysis = self.market_analysis();
        let current_hour = (self.history.len() as u32) % 24;
        let best = analysis.best_hour;
        let hours_until = if best > current_hour {
            best - current_hour
        } else if best < current_hour {
            24 - current_hour + best
        } else {
            24
        };

        Some(hours_until)
    }

    // -- statistical helpers ------------------------------------------------

    /// Gas price at a given percentile (0-100).
    pub fn percentile(&self, pct: f64) -> u64 {
        if self.history.is_empty() {
            return 0;
        }
        let mut prices: Vec<u64> = self.history.iter().map(|e| e.gas_price).collect();
        prices.sort();
        let idx = ((pct / 100.0) * (prices.len() as f64 - 1.0))
            .round()
            .min((prices.len() - 1) as f64)
            .max(0.0) as usize;
        prices[idx]
    }

    /// Simple moving average of the last `window` entries.
    pub fn moving_average(&self, window: usize) -> f64 {
        if self.history.is_empty() || window == 0 {
            return 0.0;
        }
        let start = self.history.len().saturating_sub(window);
        let slice = &self.history[start..];
        slice.iter().map(|e| e.gas_price).sum::<u64>() as f64 / slice.len() as f64
    }

    /// Standard deviation of recent gas prices.
    pub fn volatility(&self) -> f64 {
        if self.history.len() < 2 {
            return 0.0;
        }
        let prices: Vec<f64> = self.history.iter().map(|e| e.gas_price as f64).collect();
        let mean = prices.iter().sum::<f64>() / prices.len() as f64;
        let variance =
            prices.iter().map(|p| (p - mean).powi(2)).sum::<f64>() / prices.len() as f64;
        variance.sqrt()
    }

    /// Summary statistics for the optimizer state.
    pub fn stats(&self) -> FeeOptimizerStats {
        let data_points = self.history.len();
        let avg_gas_price = if data_points > 0 {
            self.history.iter().map(|e| e.gas_price).sum::<u64>() as f64 / data_points as f64
        } else {
            0.0
        };
        let median_gas_price = self.percentile(50.0) as f64;
        let analysis = self.market_analysis();

        // Count windows that save money.
        let windows = self.optimal_windows();
        let savings_opportunities = windows.iter().filter(|w| w.recommended).count();
        let total_estimated_savings = if avg_gas_price > 0.0 {
            windows
                .iter()
                .filter(|w| w.recommended)
                .map(|w| (w.avg_savings_pct / 100.0 * avg_gas_price) as u64)
                .sum()
        } else {
            0
        };

        FeeOptimizerStats {
            data_points,
            avg_gas_price,
            median_gas_price,
            current_condition: analysis.current_condition,
            savings_opportunities,
            total_estimated_savings,
        }
    }
}

// ---------------------------------------------------------------------------
// Helper: create a test entry
// ---------------------------------------------------------------------------

#[cfg(test)]
fn make_entry(gas_price: u64, utilization: f64, tx_count: u32) -> FeeHistoryEntry {
    FeeHistoryEntry {
        timestamp: chrono::Utc::now().to_rfc3339(),
        gas_price,
        block_utilization_pct: utilization,
        tx_count,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::temp_dir;
    use std::process::id;

    fn tmp_path(name: &str) -> std::path::PathBuf {
        temp_dir().join(format!("fee_optimizer_test_{}_{}", id(), name))
    }

    fn sample_optimizer(n: usize) -> FeeOptimizer {
        let mut opt = FeeOptimizer::new();
        for i in 0..n {
            opt.record_fee(make_entry(
                100 + (i as u64 * 10),
                30.0 + (i as f64),
                50 + i as u32,
            ));
        }
        opt
    }

    #[test]
    fn test_new() {
        let opt = FeeOptimizer::new();
        assert!(opt.history.is_empty());
        assert_eq!(opt.max_history, 10_000);
    }

    #[test]
    fn test_default() {
        let opt = FeeOptimizer::default();
        assert!(opt.history.is_empty());
        assert_eq!(opt.max_history, 0); // usize default
    }

    #[test]
    fn test_record_fee() {
        let mut opt = FeeOptimizer::new();
        opt.record_fee(make_entry(100, 50.0, 10));
        assert_eq!(opt.history.len(), 1);
        assert_eq!(opt.history[0].gas_price, 100);
    }

    #[test]
    fn test_record_fee_prune() {
        let mut opt = FeeOptimizer::new();
        opt.max_history = 5;
        for i in 0..10 {
            opt.record_fee(make_entry(i, 50.0, 10));
        }
        assert_eq!(opt.history.len(), 5);
        assert_eq!(opt.history[0].gas_price, 5);
    }

    #[test]
    fn test_estimate_fees_empty() {
        let opt = FeeOptimizer::new();
        let estimates = opt.estimate_fees();
        assert_eq!(estimates.len(), 4);
        assert_eq!(estimates[0].gas_price, 0);
    }

    #[test]
    fn test_estimate_fees_with_data() {
        let opt = sample_optimizer(100);
        let estimates = opt.estimate_fees();
        assert_eq!(estimates.len(), 4);
        assert!(estimates[0].gas_price <= estimates[1].gas_price);
        assert!(estimates[1].gas_price <= estimates[2].gas_price);
        assert!(estimates[2].gas_price <= estimates[3].gas_price);
    }

    #[test]
    fn test_estimate_fees_speeds() {
        let opt = sample_optimizer(50);
        let estimates = opt.estimate_fees();
        assert_eq!(estimates[0].speed, FeeSpeed::Slow);
        assert_eq!(estimates[1].speed, FeeSpeed::Standard);
        assert_eq!(estimates[2].speed, FeeSpeed::Fast);
        assert_eq!(estimates[3].speed, FeeSpeed::Instant);
    }

    #[test]
    fn test_market_analysis_empty() {
        let opt = FeeOptimizer::new();
        let analysis = opt.market_analysis();
        assert_eq!(analysis.current_condition, MarketCondition::Low);
        assert_eq!(analysis.avg_gas_24h, 0.0);
    }

    #[test]
    fn test_market_analysis_with_data() {
        let opt = sample_optimizer(48);
        let analysis = opt.market_analysis();
        assert!(analysis.avg_gas_24h > 0.0);
        assert!(analysis.min_gas_24h <= analysis.max_gas_24h);
        assert!(analysis.best_hour < 24);
        assert!(analysis.worst_hour < 24);
    }

    #[test]
    fn test_market_analysis_congested() {
        let mut opt = FeeOptimizer::new();
        for _ in 0..20 {
            opt.record_fee(make_entry(500, 95.0, 200));
        }
        let analysis = opt.market_analysis();
        assert_eq!(analysis.current_condition, MarketCondition::Congested);
    }

    #[test]
    fn test_predict_gas_instant() {
        let opt = sample_optimizer(100);
        let gas = opt.predict_gas(15);
        let p95 = opt.percentile(95.0);
        assert_eq!(gas, p95);
    }

    #[test]
    fn test_predict_gas_slow() {
        let opt = sample_optimizer(100);
        let gas = opt.predict_gas(600);
        let p25 = opt.percentile(25.0);
        assert_eq!(gas, p25);
    }

    #[test]
    fn test_predict_gas_mid() {
        let opt = sample_optimizer(100);
        let fast = opt.predict_gas(15);
        let slow = opt.predict_gas(600);
        let mid = opt.predict_gas(300);
        assert!(mid >= slow && mid <= fast);
    }

    #[test]
    fn test_optimal_windows() {
        let opt = sample_optimizer(48);
        let windows = opt.optimal_windows();
        assert!(!windows.is_empty());
        for w in &windows {
            assert!(w.start_hour < 24);
        }
    }

    #[test]
    fn test_optimal_windows_empty() {
        let opt = FeeOptimizer::new();
        let windows = opt.optimal_windows();
        assert!(windows.is_empty());
    }

    #[test]
    fn test_should_submit_now_true() {
        let mut opt = FeeOptimizer::new();
        opt.record_fee(make_entry(100, 50.0, 10));
        assert!(opt.should_submit_now(100));
        assert!(opt.should_submit_now(200));
    }

    #[test]
    fn test_should_submit_now_false() {
        let mut opt = FeeOptimizer::new();
        opt.record_fee(make_entry(100, 50.0, 10));
        assert!(!opt.should_submit_now(50));
    }

    #[test]
    fn test_wait_recommendation_none() {
        let mut opt = FeeOptimizer::new();
        opt.record_fee(make_entry(100, 50.0, 10));
        assert!(opt.wait_recommendation(200).is_none());
    }

    #[test]
    fn test_wait_recommendation_some() {
        let mut opt = FeeOptimizer::new();
        for _ in 0..48 {
            opt.record_fee(make_entry(500, 80.0, 100));
        }
        let hours = opt.wait_recommendation(50);
        assert!(hours.is_some());
    }

    #[test]
    fn test_percentile() {
        let opt = sample_optimizer(100);
        let p0 = opt.percentile(0.0);
        let p50 = opt.percentile(50.0);
        let p100 = opt.percentile(100.0);
        assert!(p0 <= p50);
        assert!(p50 <= p100);
    }

    #[test]
    fn test_moving_average() {
        let opt = sample_optimizer(10);
        let ma = opt.moving_average(5);
        assert!(ma > 0.0);
        let ma_all = opt.moving_average(100); // window > len
        assert!(ma_all > 0.0);
    }

    #[test]
    fn test_volatility() {
        let opt = sample_optimizer(50);
        let vol = opt.volatility();
        assert!(vol > 0.0);
    }

    #[test]
    fn test_volatility_empty() {
        let opt = FeeOptimizer::new();
        assert_eq!(opt.volatility(), 0.0);
    }

    #[test]
    fn test_stats() {
        let opt = sample_optimizer(48);
        let s = opt.stats();
        assert_eq!(s.data_points, 48);
        assert!(s.avg_gas_price > 0.0);
        assert!(s.median_gas_price > 0.0);
    }

    #[test]
    fn test_save_and_load() {
        let path = tmp_path("save_load.json");
        let opt = sample_optimizer(20);
        opt.save(&path).unwrap();
        let loaded = FeeOptimizer::load(&path).unwrap();
        assert_eq!(loaded.history.len(), 20);
        assert_eq!(loaded.history[0].gas_price, opt.history[0].gas_price);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_load_or_default_missing() {
        let path = tmp_path("nonexistent.json");
        let opt = FeeOptimizer::load_or_default(&path);
        assert!(opt.history.is_empty());
    }
}
