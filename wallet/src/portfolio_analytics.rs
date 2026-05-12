//! Portfolio analytics with risk metrics for the EvaporChain wallet.
//!
//! Provides:
//! - Daily return tracking per token and benchmark
//! - Asset allocation weights
//! - Risk metrics: Sharpe, Sortino, max drawdown, volatility, VaR, beta
//! - Correlation analysis (pairwise and matrix)
//! - Diversification scoring (HHI-based)
//! - Performance ranking (best/worst performers)

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum PortfolioAnalyticsError {
    #[error("token not found: {0}")]
    TokenNotFound(String),
    #[error("insufficient data: need at least {needed}, have {have}")]
    InsufficientData { needed: usize, have: usize },
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

// ──────────────────────────── Types ──────────────────────────────────────

/// A single daily return data point.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyReturn {
    pub date: String,
    pub value: f64,
    pub return_pct: f64,
}

/// Asset allocation entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetAllocation {
    pub token: String,
    pub value: u64,
    pub weight_pct: f64,
}

/// Comprehensive risk metrics for a token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskMetrics {
    pub sharpe_ratio: f64,
    pub sortino_ratio: f64,
    pub max_drawdown_pct: f64,
    pub volatility: f64,
    pub beta: f64,
    pub var_95: f64,
}

/// Pairwise correlation entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorrelationEntry {
    pub token_a: String,
    pub token_b: String,
    pub correlation: f64,
}

/// Portfolio diversification score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiversificationScore {
    /// Overall diversification score 0-100.
    pub score: f64,
    /// Herfindahl-Hirschman Index.
    pub hhi: f64,
    /// Effective number of assets (1/HHI).
    pub effective_assets: f64,
    /// Concentration risk level: Low / Medium / High.
    pub concentration_risk: String,
}

/// Summary statistics for the analytics store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyticsStats {
    pub tokens_tracked: usize,
    pub data_points: usize,
    pub date_range_days: u32,
    pub total_portfolio_value: u64,
    pub best_performer: Option<String>,
    pub worst_performer: Option<String>,
}

// ──────────────────────────── Store ─────────────────────────────────────

/// Main portfolio analytics store.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PortfolioAnalytics {
    /// Token -> daily return history.
    pub daily_values: HashMap<String, Vec<DailyReturn>>,
    /// Benchmark daily returns.
    pub benchmark_returns: Vec<DailyReturn>,
}

impl PortfolioAnalytics {
    /// Create a new empty analytics store.
    pub fn new() -> Self {
        Self::default()
    }

    // ──────────── Recording ──────────────────────────────────────────

    /// Record a value for a token on a given date.
    /// Computes return_pct from the previous entry if available.
    pub fn record_value(&mut self, token: &str, date: &str, value: f64) {
        let entries = self.daily_values.entry(token.to_string()).or_default();
        let return_pct = if let Some(prev) = entries.last() {
            if prev.value != 0.0 {
                ((value - prev.value) / prev.value) * 100.0
            } else {
                0.0
            }
        } else {
            0.0
        };
        entries.push(DailyReturn {
            date: date.to_string(),
            value,
            return_pct,
        });
    }

    /// Record a benchmark value on a given date.
    pub fn record_benchmark(&mut self, date: &str, value: f64) {
        let return_pct = if let Some(prev) = self.benchmark_returns.last() {
            if prev.value != 0.0 {
                ((value - prev.value) / prev.value) * 100.0
            } else {
                0.0
            }
        } else {
            0.0
        };
        self.benchmark_returns.push(DailyReturn {
            date: date.to_string(),
            value,
            return_pct,
        });
    }

    // ──────────── Allocation ─────────────────────────────────────────

    /// Compute asset allocation weights from holdings.
    pub fn asset_allocation(&self, holdings: &HashMap<String, u64>) -> Vec<AssetAllocation> {
        let total: u64 = holdings.values().sum();
        if total == 0 {
            return holdings
                .iter()
                .map(|(token, &value)| AssetAllocation {
                    token: token.clone(),
                    value,
                    weight_pct: 0.0,
                })
                .collect();
        }
        let mut allocs: Vec<AssetAllocation> = holdings
            .iter()
            .map(|(token, &value)| AssetAllocation {
                token: token.clone(),
                value,
                weight_pct: (value as f64 / total as f64) * 100.0,
            })
            .collect();
        allocs.sort_by(|a, b| b.weight_pct.partial_cmp(&a.weight_pct).unwrap());
        allocs
    }

    // ──────────── Risk metrics ───────────────────────────────────────

    fn get_returns(&self, token: &str) -> Result<Vec<f64>, PortfolioAnalyticsError> {
        let entries = self
            .daily_values
            .get(token)
            .ok_or_else(|| PortfolioAnalyticsError::TokenNotFound(token.to_string()))?;
        if entries.len() < 2 {
            return Err(PortfolioAnalyticsError::InsufficientData {
                needed: 2,
                have: entries.len(),
            });
        }
        Ok(entries.iter().map(|e| e.return_pct).collect())
    }

    fn mean(values: &[f64]) -> f64 {
        if values.is_empty() {
            return 0.0;
        }
        values.iter().sum::<f64>() / values.len() as f64
    }

    fn std_dev(values: &[f64]) -> f64 {
        if values.len() < 2 {
            return 0.0;
        }
        let avg = Self::mean(values);
        let variance =
            values.iter().map(|v| (v - avg).powi(2)).sum::<f64>() / (values.len() - 1) as f64;
        variance.sqrt()
    }

    /// Sharpe ratio = (mean_return - risk_free_rate) / std_dev.
    pub fn sharpe_ratio(
        &self,
        token: &str,
        risk_free_rate: f64,
    ) -> Result<f64, PortfolioAnalyticsError> {
        let returns = self.get_returns(token)?;
        let avg = Self::mean(&returns);
        let sd = Self::std_dev(&returns);
        if sd == 0.0 {
            return Ok(0.0);
        }
        Ok((avg - risk_free_rate) / sd)
    }

    /// Sortino ratio — uses only downside deviation.
    pub fn sortino_ratio(
        &self,
        token: &str,
        risk_free_rate: f64,
    ) -> Result<f64, PortfolioAnalyticsError> {
        let returns = self.get_returns(token)?;
        let avg = Self::mean(&returns);
        let downside: Vec<f64> = returns
            .iter()
            .filter(|&&r| r < risk_free_rate)
            .cloned()
            .collect();
        if downside.is_empty() {
            return Ok(0.0);
        }
        let downside_var = downside
            .iter()
            .map(|r| (r - risk_free_rate).powi(2))
            .sum::<f64>()
            / downside.len() as f64;
        let downside_dev = downside_var.sqrt();
        if downside_dev == 0.0 {
            return Ok(0.0);
        }
        Ok((avg - risk_free_rate) / downside_dev)
    }

    /// Maximum drawdown as a percentage (peak-to-trough).
    pub fn max_drawdown(&self, token: &str) -> Result<f64, PortfolioAnalyticsError> {
        let entries = self
            .daily_values
            .get(token)
            .ok_or_else(|| PortfolioAnalyticsError::TokenNotFound(token.to_string()))?;
        if entries.len() < 2 {
            return Err(PortfolioAnalyticsError::InsufficientData {
                needed: 2,
                have: entries.len(),
            });
        }
        let mut peak = entries[0].value;
        let mut max_dd = 0.0_f64;
        for entry in entries.iter() {
            if entry.value > peak {
                peak = entry.value;
            }
            if peak > 0.0 {
                let dd = (peak - entry.value) / peak * 100.0;
                if dd > max_dd {
                    max_dd = dd;
                }
            }
        }
        Ok(max_dd)
    }

    /// Volatility (standard deviation of returns).
    pub fn volatility(&self, token: &str) -> Result<f64, PortfolioAnalyticsError> {
        let returns = self.get_returns(token)?;
        Ok(Self::std_dev(&returns))
    }

    /// Pearson correlation between two tokens' returns.
    pub fn correlation(
        &self,
        token_a: &str,
        token_b: &str,
    ) -> Result<f64, PortfolioAnalyticsError> {
        let returns_a = self.get_returns(token_a)?;
        let returns_b = self.get_returns(token_b)?;
        let n = returns_a.len().min(returns_b.len());
        if n < 2 {
            return Err(PortfolioAnalyticsError::InsufficientData { needed: 2, have: n });
        }
        let a = &returns_a[..n];
        let b = &returns_b[..n];
        let mean_a = Self::mean(a);
        let mean_b = Self::mean(b);
        let mut cov = 0.0;
        let mut var_a = 0.0;
        let mut var_b = 0.0;
        for i in 0..n {
            let da = a[i] - mean_a;
            let db = b[i] - mean_b;
            cov += da * db;
            var_a += da * da;
            var_b += db * db;
        }
        let denom = (var_a * var_b).sqrt();
        if denom == 0.0 {
            return Ok(0.0);
        }
        Ok(cov / denom)
    }

    /// Correlation matrix for a set of tokens.
    pub fn correlation_matrix(
        &self,
        tokens: &[&str],
    ) -> Result<Vec<CorrelationEntry>, PortfolioAnalyticsError> {
        let mut entries = Vec::new();
        for i in 0..tokens.len() {
            for j in (i + 1)..tokens.len() {
                let corr = self.correlation(tokens[i], tokens[j])?;
                entries.push(CorrelationEntry {
                    token_a: tokens[i].to_string(),
                    token_b: tokens[j].to_string(),
                    correlation: corr,
                });
            }
        }
        Ok(entries)
    }

    /// Full risk metrics for a token.
    pub fn risk_metrics(
        &self,
        token: &str,
        risk_free_rate: f64,
    ) -> Result<RiskMetrics, PortfolioAnalyticsError> {
        let returns = self.get_returns(token)?;
        let sharpe = self.sharpe_ratio(token, risk_free_rate)?;
        let sortino = self.sortino_ratio(token, risk_free_rate)?;
        let max_dd = self.max_drawdown(token)?;
        let vol = Self::std_dev(&returns);

        // Beta = cov(token, benchmark) / var(benchmark)
        let beta = if self.benchmark_returns.len() >= 2 {
            let bench_returns: Vec<f64> = self
                .benchmark_returns
                .iter()
                .map(|e| e.return_pct)
                .collect();
            let n = returns.len().min(bench_returns.len());
            if n >= 2 {
                let a = &returns[..n];
                let b = &bench_returns[..n];
                let mean_a = Self::mean(a);
                let mean_b = Self::mean(b);
                let mut cov = 0.0;
                let mut var_b = 0.0;
                for i in 0..n {
                    let da = a[i] - mean_a;
                    let db = b[i] - mean_b;
                    cov += da * db;
                    var_b += db * db;
                }
                if var_b == 0.0 {
                    1.0
                } else {
                    cov / var_b
                }
            } else {
                1.0
            }
        } else {
            1.0
        };

        // VaR 95% — parametric (assuming normal distribution)
        let mean_ret = Self::mean(&returns);
        let var_95 = mean_ret - 1.645 * vol;

        Ok(RiskMetrics {
            sharpe_ratio: sharpe,
            sortino_ratio: sortino,
            max_drawdown_pct: max_dd,
            volatility: vol,
            beta,
            var_95,
        })
    }

    // ──────────── Diversification ────────────────────────────────────

    /// Compute diversification score from holdings using the HHI.
    pub fn diversification_score(&self, holdings: &HashMap<String, u64>) -> DiversificationScore {
        let total: u64 = holdings.values().sum();
        if total == 0 || holdings.is_empty() {
            return DiversificationScore {
                score: 0.0,
                hhi: 10000.0,
                effective_assets: 0.0,
                concentration_risk: "High".to_string(),
            };
        }
        let weights: Vec<f64> = holdings
            .values()
            .map(|&v| v as f64 / total as f64)
            .collect();
        let hhi: f64 = weights.iter().map(|w| (w * 100.0).powi(2)).sum();
        let effective_assets = if hhi > 0.0 { 10000.0 / hhi } else { 0.0 };
        // Score: 100 means perfectly diversified (HHI -> min), 0 means fully concentrated
        let n = holdings.len() as f64;
        let score = if n <= 1.0 {
            0.0
        } else {
            let min_hhi = 10000.0 / n;
            if hhi <= min_hhi {
                100.0
            } else {
                ((10000.0 - hhi) / (10000.0 - min_hhi) * 100.0).clamp(0.0, 100.0)
            }
        };
        let concentration_risk = if score >= 70.0 {
            "Low".to_string()
        } else if score >= 40.0 {
            "Medium".to_string()
        } else {
            "High".to_string()
        };
        DiversificationScore {
            score,
            hhi,
            effective_assets,
            concentration_risk,
        }
    }

    // ──────────── Performance ────────────────────────────────────────

    /// Total return percentage for a token (first value to last value).
    fn total_return(&self, token: &str) -> Option<f64> {
        let entries = self.daily_values.get(token)?;
        if entries.len() < 2 {
            return None;
        }
        let first = entries.first()?.value;
        let last = entries.last()?.value;
        if first == 0.0 {
            return None;
        }
        Some(((last - first) / first) * 100.0)
    }

    /// Best performer by total return.
    pub fn best_performer(&self) -> Option<(String, f64)> {
        self.daily_values
            .keys()
            .filter_map(|token| self.total_return(token).map(|r| (token.clone(), r)))
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
    }

    /// Worst performer by total return.
    pub fn worst_performer(&self) -> Option<(String, f64)> {
        self.daily_values
            .keys()
            .filter_map(|token| self.total_return(token).map(|r| (token.clone(), r)))
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
    }

    // ──────────── Stats ──────────────────────────────────────────────

    /// Summary statistics for the analytics store.
    pub fn stats(&self, holdings: &HashMap<String, u64>) -> AnalyticsStats {
        let tokens_tracked = self.daily_values.len();
        let data_points: usize = self.daily_values.values().map(|v| v.len()).sum();

        // Compute date range in days (rough: count distinct dates across all tokens)
        let mut all_dates: Vec<&str> = self
            .daily_values
            .values()
            .flat_map(|v| v.iter().map(|e| e.date.as_str()))
            .collect();
        all_dates.sort_unstable();
        all_dates.dedup();
        let date_range_days = if all_dates.len() > 1 {
            all_dates.len() as u32 - 1
        } else {
            0
        };

        let total_portfolio_value: u64 = holdings.values().sum();
        let best = self.best_performer().map(|(t, _)| t);
        let worst = self.worst_performer().map(|(t, _)| t);

        AnalyticsStats {
            tokens_tracked,
            data_points,
            date_range_days,
            total_portfolio_value,
            best_performer: best,
            worst_performer: worst,
        }
    }

    // ──────────── Persistence ────────────────────────────────────────

    /// Load from a JSON file.
    pub fn load(path: &Path) -> Result<Self, PortfolioAnalyticsError> {
        let data = std::fs::read_to_string(path)?;
        let store: PortfolioAnalytics = serde_json::from_str(&data)?;
        Ok(store)
    }

    /// Save to a JSON file.
    pub fn save(&self, path: &Path) -> Result<(), PortfolioAnalyticsError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Load from file, or return a default instance if loading fails.
    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ──────────────────────────── Tests ─────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("evaporchain_test_{}_{}", std::process::id(), name))
    }

    fn make_analytics() -> PortfolioAnalytics {
        let mut pa = PortfolioAnalytics::new();
        pa.record_value("EVP", "2026-01-01", 100.0);
        pa.record_value("EVP", "2026-01-02", 105.0);
        pa.record_value("EVP", "2026-01-03", 103.0);
        pa.record_value("EVP", "2026-01-04", 110.0);
        pa.record_value("EVP", "2026-01-05", 108.0);

        pa.record_value("SOL", "2026-01-01", 50.0);
        pa.record_value("SOL", "2026-01-02", 48.0);
        pa.record_value("SOL", "2026-01-03", 52.0);
        pa.record_value("SOL", "2026-01-04", 47.0);
        pa.record_value("SOL", "2026-01-05", 45.0);

        pa.record_benchmark("2026-01-01", 1000.0);
        pa.record_benchmark("2026-01-02", 1020.0);
        pa.record_benchmark("2026-01-03", 1010.0);
        pa.record_benchmark("2026-01-04", 1030.0);
        pa.record_benchmark("2026-01-05", 1025.0);

        pa
    }

    #[test]
    fn test_new() {
        let pa = PortfolioAnalytics::new();
        assert!(pa.daily_values.is_empty());
        assert!(pa.benchmark_returns.is_empty());
    }

    #[test]
    fn test_record_value() {
        let mut pa = PortfolioAnalytics::new();
        pa.record_value("EVP", "2026-01-01", 100.0);
        pa.record_value("EVP", "2026-01-02", 110.0);
        let entries = &pa.daily_values["EVP"];
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].return_pct, 0.0);
        assert!((entries[1].return_pct - 10.0).abs() < 0.001);
    }

    #[test]
    fn test_record_benchmark() {
        let mut pa = PortfolioAnalytics::new();
        pa.record_benchmark("2026-01-01", 1000.0);
        pa.record_benchmark("2026-01-02", 1050.0);
        assert_eq!(pa.benchmark_returns.len(), 2);
        assert!((pa.benchmark_returns[1].return_pct - 5.0).abs() < 0.001);
    }

    #[test]
    fn test_asset_allocation() {
        let pa = PortfolioAnalytics::new();
        let mut holdings = HashMap::new();
        holdings.insert("EVP".to_string(), 7000);
        holdings.insert("SOL".to_string(), 3000);
        let alloc = pa.asset_allocation(&holdings);
        assert_eq!(alloc.len(), 2);
        assert_eq!(alloc[0].token, "EVP");
        assert!((alloc[0].weight_pct - 70.0).abs() < 0.001);
        assert!((alloc[1].weight_pct - 30.0).abs() < 0.001);
    }

    #[test]
    fn test_asset_allocation_empty() {
        let pa = PortfolioAnalytics::new();
        let holdings = HashMap::new();
        let alloc = pa.asset_allocation(&holdings);
        assert!(alloc.is_empty());
    }

    #[test]
    fn test_sharpe_ratio() {
        let pa = make_analytics();
        let sharpe = pa.sharpe_ratio("EVP", 0.0).unwrap();
        // Should be a finite number
        assert!(sharpe.is_finite());
    }

    #[test]
    fn test_sharpe_ratio_token_not_found() {
        let pa = make_analytics();
        let result = pa.sharpe_ratio("MISSING", 0.0);
        assert!(result.is_err());
    }

    #[test]
    fn test_sortino_ratio() {
        let pa = make_analytics();
        let sortino = pa.sortino_ratio("EVP", 0.0).unwrap();
        assert!(sortino.is_finite());
    }

    #[test]
    fn test_max_drawdown() {
        let pa = make_analytics();
        let dd = pa.max_drawdown("EVP").unwrap();
        // EVP went 105 -> 103, so there's a drawdown
        assert!(dd > 0.0);
        assert!(dd < 100.0);
    }

    #[test]
    fn test_max_drawdown_token_not_found() {
        let pa = make_analytics();
        let result = pa.max_drawdown("MISSING");
        assert!(result.is_err());
    }

    #[test]
    fn test_volatility() {
        let pa = make_analytics();
        let vol = pa.volatility("EVP").unwrap();
        assert!(vol > 0.0);
        assert!(vol.is_finite());
    }

    #[test]
    fn test_correlation() {
        let pa = make_analytics();
        let corr = pa.correlation("EVP", "SOL").unwrap();
        assert!((-1.0..=1.0).contains(&corr));
    }

    #[test]
    fn test_correlation_matrix() {
        let pa = make_analytics();
        let matrix = pa.correlation_matrix(&["EVP", "SOL"]).unwrap();
        assert_eq!(matrix.len(), 1);
        assert_eq!(matrix[0].token_a, "EVP");
        assert_eq!(matrix[0].token_b, "SOL");
        assert!(matrix[0].correlation >= -1.0 && matrix[0].correlation <= 1.0);
    }

    #[test]
    fn test_risk_metrics() {
        let pa = make_analytics();
        let rm = pa.risk_metrics("EVP", 0.0).unwrap();
        assert!(rm.sharpe_ratio.is_finite());
        assert!(rm.sortino_ratio.is_finite());
        assert!(rm.max_drawdown_pct >= 0.0);
        assert!(rm.volatility >= 0.0);
        assert!(rm.beta.is_finite());
        assert!(rm.var_95.is_finite());
    }

    #[test]
    fn test_diversification_score() {
        let pa = PortfolioAnalytics::new();
        let mut holdings = HashMap::new();
        holdings.insert("EVP".to_string(), 5000);
        holdings.insert("SOL".to_string(), 5000);
        let ds = pa.diversification_score(&holdings);
        assert!((ds.score - 100.0).abs() < 0.01); // Equal weights = max diversification
        assert!((ds.hhi - 5000.0).abs() < 0.01);
        assert!((ds.effective_assets - 2.0).abs() < 0.01);
        assert_eq!(ds.concentration_risk, "Low");
    }

    #[test]
    fn test_diversification_score_concentrated() {
        let pa = PortfolioAnalytics::new();
        let mut holdings = HashMap::new();
        holdings.insert("EVP".to_string(), 10000);
        let ds = pa.diversification_score(&holdings);
        assert!((ds.hhi - 10000.0).abs() < 0.01);
        assert_eq!(ds.concentration_risk, "High");
    }

    #[test]
    fn test_diversification_score_empty() {
        let pa = PortfolioAnalytics::new();
        let holdings = HashMap::new();
        let ds = pa.diversification_score(&holdings);
        assert_eq!(ds.score, 0.0);
        assert_eq!(ds.concentration_risk, "High");
    }

    #[test]
    fn test_best_performer() {
        let pa = make_analytics();
        let (token, ret) = pa.best_performer().unwrap();
        assert_eq!(token, "EVP"); // EVP went 100 -> 108 = +8%
        assert!(ret > 0.0);
    }

    #[test]
    fn test_worst_performer() {
        let pa = make_analytics();
        let (token, ret) = pa.worst_performer().unwrap();
        assert_eq!(token, "SOL"); // SOL went 50 -> 45 = -10%
        assert!(ret < 0.0);
    }

    #[test]
    fn test_stats() {
        let pa = make_analytics();
        let mut holdings = HashMap::new();
        holdings.insert("EVP".to_string(), 5000);
        holdings.insert("SOL".to_string(), 3000);
        let s = pa.stats(&holdings);
        assert_eq!(s.tokens_tracked, 2);
        assert_eq!(s.data_points, 10); // 5 per token
        assert_eq!(s.date_range_days, 4); // 5 unique dates - 1
        assert_eq!(s.total_portfolio_value, 8000);
        assert_eq!(s.best_performer.as_deref(), Some("EVP"));
        assert_eq!(s.worst_performer.as_deref(), Some("SOL"));
    }

    #[test]
    fn test_save_and_load() {
        let pa = make_analytics();
        let path = temp_path("portfolio_analytics.json");
        pa.save(&path).unwrap();
        let loaded = PortfolioAnalytics::load(&path).unwrap();
        assert_eq!(loaded.daily_values.len(), 2);
        assert_eq!(loaded.benchmark_returns.len(), 5);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_load_or_default_missing() {
        let path = temp_path("nonexistent_pa.json");
        let pa = PortfolioAnalytics::load_or_default(&path);
        assert!(pa.daily_values.is_empty());
    }

    #[test]
    fn test_insufficient_data() {
        let mut pa = PortfolioAnalytics::new();
        pa.record_value("ONLY", "2026-01-01", 100.0);
        let result = pa.sharpe_ratio("ONLY", 0.0);
        assert!(result.is_err());
    }
}
