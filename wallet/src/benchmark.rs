// wallet/src/benchmark.rs — Performance benchmarks for wallet operations
//
// Micro-benchmark framework: measures signing, hashing, encryption,
// serialization, and address derivation. Reports min/max/mean/median/p99.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BenchmarkError {
    #[error("benchmark not found: {0}")]
    NotFound(String),
    #[error("benchmark failed: {0}")]
    Failed(String),
    #[error("invalid config: {0}")]
    InvalidConfig(String),
}

// ── Result types ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchResult {
    pub name: String,
    pub iterations: usize,
    pub total_ns: u128,
    pub min_ns: u128,
    pub max_ns: u128,
    pub mean_ns: u128,
    pub median_ns: u128,
    pub p99_ns: u128,
    pub ops_per_sec: f64,
}

impl BenchResult {
    pub fn total_duration(&self) -> Duration {
        Duration::from_nanos(self.total_ns as u64)
    }

    pub fn mean_duration(&self) -> Duration {
        Duration::from_nanos(self.mean_ns as u64)
    }

    pub fn to_report_line(&self) -> String {
        format!(
            "{:<30} {:>8} iters  mean={:>10}  min={:>10}  max={:>10}  p99={:>10}  {:.0} ops/s",
            self.name,
            self.iterations,
            format_ns(self.mean_ns),
            format_ns(self.min_ns),
            format_ns(self.max_ns),
            format_ns(self.p99_ns),
            self.ops_per_sec,
        )
    }
}

pub fn format_ns(ns: u128) -> String {
    if ns >= 1_000_000_000 {
        format!("{:.2}s", ns as f64 / 1e9)
    } else if ns >= 1_000_000 {
        format!("{:.2}ms", ns as f64 / 1e6)
    } else if ns >= 1_000 {
        format!("{:.2}us", ns as f64 / 1e3)
    } else {
        format!("{}ns", ns)
    }
}

// ── Benchmark runner ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchConfig {
    pub warmup_iterations: usize,
    pub iterations: usize,
}

impl Default for BenchConfig {
    fn default() -> Self {
        Self {
            warmup_iterations: 10,
            iterations: 100,
        }
    }
}

impl BenchConfig {
    pub fn quick() -> Self {
        Self {
            warmup_iterations: 2,
            iterations: 20,
        }
    }

    pub fn thorough() -> Self {
        Self {
            warmup_iterations: 50,
            iterations: 1000,
        }
    }

    pub fn validate(&self) -> Result<(), BenchmarkError> {
        if self.iterations == 0 {
            return Err(BenchmarkError::InvalidConfig(
                "iterations must be > 0".into(),
            ));
        }
        Ok(())
    }
}

/// Run a single benchmark: execute `f` for warmup, then measure `iterations` runs.
pub fn run_bench<F>(
    name: &str,
    config: &BenchConfig,
    mut f: F,
) -> Result<BenchResult, BenchmarkError>
where
    F: FnMut(),
{
    config.validate()?;

    // Warmup
    for _ in 0..config.warmup_iterations {
        f();
    }

    // Measure
    let mut timings = Vec::with_capacity(config.iterations);
    for _ in 0..config.iterations {
        let start = Instant::now();
        f();
        let elapsed = start.elapsed().as_nanos();
        timings.push(elapsed);
    }

    Ok(compute_stats(name, &mut timings))
}

fn compute_stats(name: &str, timings: &mut [u128]) -> BenchResult {
    timings.sort();
    let n = timings.len();
    let total: u128 = timings.iter().sum();
    let min = timings[0];
    let max = timings[n - 1];
    let mean = total / n as u128;
    let median = if n.is_multiple_of(2) {
        (timings[n / 2 - 1] + timings[n / 2]) / 2
    } else {
        timings[n / 2]
    };
    let p99_idx = ((n as f64) * 0.99).ceil() as usize - 1;
    let p99 = timings[p99_idx.min(n - 1)];
    let ops_per_sec = if mean > 0 {
        1_000_000_000.0 / mean as f64
    } else {
        f64::INFINITY
    };

    BenchResult {
        name: name.to_string(),
        iterations: n,
        total_ns: total,
        min_ns: min,
        max_ns: max,
        mean_ns: mean,
        median_ns: median,
        p99_ns: p99,
        ops_per_sec,
    }
}

// ── Benchmark suite ───────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchSuite {
    pub name: String,
    pub results: Vec<BenchResult>,
    pub timestamp: String,
    pub config: BenchConfig,
}

impl BenchSuite {
    pub fn new(name: &str, config: BenchConfig) -> Self {
        Self {
            name: name.to_string(),
            results: Vec::new(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            config,
        }
    }

    pub fn add(&mut self, result: BenchResult) {
        self.results.push(result);
    }

    pub fn run<F>(&mut self, name: &str, f: F) -> Result<&BenchResult, BenchmarkError>
    where
        F: FnMut(),
    {
        let result = run_bench(name, &self.config, f)?;
        self.results.push(result);
        Ok(self.results.last().unwrap())
    }

    pub fn to_report(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("=== {} ===\n", self.name));
        out.push_str(&format!("Timestamp: {}\n", self.timestamp));
        out.push_str(&format!(
            "Config: {} warmup + {} iterations\n\n",
            self.config.warmup_iterations, self.config.iterations
        ));
        for r in &self.results {
            out.push_str(&r.to_report_line());
            out.push('\n');
        }
        out
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn fastest(&self) -> Option<&BenchResult> {
        self.results.iter().min_by_key(|r| r.mean_ns)
    }

    pub fn slowest(&self) -> Option<&BenchResult> {
        self.results.iter().max_by_key(|r| r.mean_ns)
    }

    pub fn total_time(&self) -> Duration {
        let total_ns: u128 = self.results.iter().map(|r| r.total_ns).sum();
        Duration::from_nanos(total_ns as u64)
    }
}

// ── Comparison ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchComparison {
    pub name: String,
    pub baseline: BenchResult,
    pub current: BenchResult,
    pub speedup: f64,     // >1 means current is faster
    pub regression: bool, // true if current is slower by > threshold
}

pub fn compare(
    baseline: &BenchResult,
    current: &BenchResult,
    regression_threshold: f64,
) -> BenchComparison {
    let speedup = if current.mean_ns > 0 {
        baseline.mean_ns as f64 / current.mean_ns as f64
    } else {
        f64::INFINITY
    };
    BenchComparison {
        name: current.name.clone(),
        baseline: baseline.clone(),
        current: current.clone(),
        speedup,
        regression: speedup < (1.0 - regression_threshold),
    }
}

// ── Built-in wallet benchmarks ────────────────────────────────

/// Run built-in wallet operation benchmarks
pub fn run_wallet_benchmarks(config: &BenchConfig) -> Result<BenchSuite, BenchmarkError> {
    let mut suite = BenchSuite::new("EvaporChain Wallet Benchmarks", config.clone());

    // BLAKE3 hashing benchmark
    suite.run("blake3_hash_256b", || {
        let data = [0u8; 256];
        let _ = blake3::hash(&data);
    })?;

    suite.run("blake3_hash_1kb", || {
        let data = [0u8; 1024];
        let _ = blake3::hash(&data);
    })?;

    suite.run("blake3_hash_4kb", || {
        let data = [0u8; 4096];
        let _ = blake3::hash(&data);
    })?;

    // Address derivation
    suite.run("address_derivation", || {
        let fake_pk = vec![0u8; 2592]; // ML-DSA-65 public key size
        let _ = crate::address::derive_address(&fake_pk);
    })?;

    // JSON serialization benchmark
    suite.run("json_serialize_tx", || {
        let tx = serde_json::json!({
            "type": "transfer",
            "from": "evap1abc",
            "to": "evap1def",
            "amount": 1000,
            "nonce": 42,
            "gas_limit": 21000,
        });
        let _ = serde_json::to_string(&tx).unwrap();
    })?;

    suite.run("json_deserialize_tx", || {
        let data = r#"{"type":"transfer","from":"evap1abc","to":"evap1def","amount":1000,"nonce":42,"gas_limit":21000}"#;
        let _: serde_json::Value = serde_json::from_str(data).unwrap();
    })?;

    // Hex encoding/decoding
    suite.run("hex_encode_64b", || {
        let data = [0xABu8; 64];
        let _ = hex::encode(data);
    })?;

    suite.run("hex_decode_128char", || {
        let hex_str = "ab".repeat(64);
        let _ = hex::decode(&hex_str).unwrap();
    })?;

    Ok(suite)
}

/// Saved benchmark history for regression tracking
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BenchHistory {
    pub runs: Vec<BenchSuite>,
}

impl BenchHistory {
    pub fn new() -> Self {
        Self { runs: Vec::new() }
    }

    pub fn add_run(&mut self, suite: BenchSuite) {
        self.runs.push(suite);
    }

    pub fn latest(&self) -> Option<&BenchSuite> {
        self.runs.last()
    }

    pub fn get_baseline(&self, name: &str) -> Option<&BenchResult> {
        if let Some(first) = self.runs.first() {
            first.results.iter().find(|r| r.name == name)
        } else {
            None
        }
    }

    pub fn trend(&self, bench_name: &str) -> Vec<(String, u128)> {
        self.runs
            .iter()
            .filter_map(|suite| {
                suite
                    .results
                    .iter()
                    .find(|r| r.name == bench_name)
                    .map(|r| (suite.timestamp.clone(), r.mean_ns))
            })
            .collect()
    }

    pub fn check_regressions(&self, threshold: f64) -> Vec<BenchComparison> {
        if self.runs.len() < 2 {
            return Vec::new();
        }
        let baseline = &self.runs[0];
        let latest = self.runs.last().unwrap();

        let baseline_map: BTreeMap<&str, &BenchResult> = baseline
            .results
            .iter()
            .map(|r| (r.name.as_str(), r))
            .collect();

        latest
            .results
            .iter()
            .filter_map(|current| {
                baseline_map
                    .get(current.name.as_str())
                    .map(|base| compare(base, current, threshold))
            })
            .filter(|c| c.regression)
            .collect()
    }

    pub fn save(&self, path: &std::path::Path) -> Result<(), BenchmarkError> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| BenchmarkError::Failed(e.to_string()))?;
        std::fs::write(path, json).map_err(|e| BenchmarkError::Failed(e.to_string()))
    }

    pub fn load(path: &std::path::Path) -> Result<Self, BenchmarkError> {
        let data =
            std::fs::read_to_string(path).map_err(|e| BenchmarkError::Failed(e.to_string()))?;
        serde_json::from_str(&data).map_err(|e| BenchmarkError::Failed(e.to_string()))
    }
}

// ── Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bench_config_default() {
        let c = BenchConfig::default();
        assert_eq!(c.warmup_iterations, 10);
        assert_eq!(c.iterations, 100);
        assert!(c.validate().is_ok());
    }

    #[test]
    fn test_bench_config_quick() {
        let c = BenchConfig::quick();
        assert_eq!(c.iterations, 20);
    }

    #[test]
    fn test_bench_config_zero_iterations_rejected() {
        let c = BenchConfig {
            warmup_iterations: 0,
            iterations: 0,
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn test_run_bench_basic() {
        let config = BenchConfig::quick();
        let result = run_bench("noop", &config, || {}).unwrap();
        assert_eq!(result.name, "noop");
        assert_eq!(result.iterations, 20);
        assert!(result.min_ns <= result.mean_ns);
        assert!(result.mean_ns <= result.max_ns);
        assert!(result.ops_per_sec > 0.0);
    }

    #[test]
    fn test_run_bench_with_work() {
        let config = BenchConfig::quick();
        let result = run_bench("hash", &config, || {
            let _ = blake3::hash(b"benchmark test data");
        })
        .unwrap();
        assert!(result.total_ns > 0);
        assert!(result.median_ns > 0);
    }

    #[test]
    fn test_bench_result_durations() {
        let config = BenchConfig::quick();
        let result = run_bench("dur", &config, || {}).unwrap();
        assert!(result.total_duration() >= Duration::ZERO);
        assert!(result.mean_duration() >= Duration::ZERO);
    }

    #[test]
    fn test_bench_result_report_line() {
        let config = BenchConfig::quick();
        let result = run_bench("report_test", &config, || {}).unwrap();
        let line = result.to_report_line();
        assert!(line.contains("report_test"));
        assert!(line.contains("ops/s"));
    }

    #[test]
    fn test_bench_suite() {
        let config = BenchConfig::quick();
        let mut suite = BenchSuite::new("test suite", config);
        suite.run("bench_a", || {}).unwrap();
        suite
            .run("bench_b", || {
                let _ = 1 + 1;
            })
            .unwrap();
        assert_eq!(suite.results.len(), 2);
        assert!(suite.fastest().is_some());
        assert!(suite.slowest().is_some());
    }

    #[test]
    fn test_bench_suite_report() {
        let config = BenchConfig::quick();
        let mut suite = BenchSuite::new("report", config);
        suite.run("op", || {}).unwrap();
        let report = suite.to_report();
        assert!(report.contains("=== report ==="));
        assert!(report.contains("op"));
    }

    #[test]
    fn test_bench_suite_json() {
        let config = BenchConfig::quick();
        let mut suite = BenchSuite::new("json", config);
        suite.run("x", || {}).unwrap();
        let json = suite.to_json();
        assert!(json.contains("\"name\": \"json\""));
    }

    #[test]
    fn test_bench_suite_total_time() {
        let config = BenchConfig::quick();
        let mut suite = BenchSuite::new("tt", config);
        suite.run("a", || {}).unwrap();
        assert!(suite.total_time() >= Duration::ZERO);
    }

    #[test]
    fn test_compare_faster() {
        let baseline = BenchResult {
            name: "test".into(),
            iterations: 100,
            total_ns: 10000,
            min_ns: 80,
            max_ns: 120,
            mean_ns: 100,
            median_ns: 100,
            p99_ns: 115,
            ops_per_sec: 10_000_000.0,
        };
        let current = BenchResult {
            mean_ns: 50,
            ..baseline.clone()
        };
        let cmp = compare(&baseline, &current, 0.1);
        assert!(cmp.speedup > 1.0);
        assert!(!cmp.regression);
    }

    #[test]
    fn test_compare_regression() {
        let baseline = BenchResult {
            name: "test".into(),
            iterations: 100,
            total_ns: 10000,
            min_ns: 80,
            max_ns: 120,
            mean_ns: 100,
            median_ns: 100,
            p99_ns: 115,
            ops_per_sec: 10_000_000.0,
        };
        let current = BenchResult {
            mean_ns: 200,
            ..baseline.clone()
        };
        let cmp = compare(&baseline, &current, 0.1);
        assert!(cmp.speedup < 1.0);
        assert!(cmp.regression);
    }

    #[test]
    fn test_bench_history() {
        let mut history = BenchHistory::new();
        assert!(history.latest().is_none());

        let config = BenchConfig::quick();
        let mut suite = BenchSuite::new("run1", config);
        suite.run("op", || {}).unwrap();
        history.add_run(suite);

        assert!(history.latest().is_some());
        assert!(history.get_baseline("op").is_some());
        assert!(history.get_baseline("nope").is_none());
    }

    #[test]
    fn test_bench_history_trend() {
        let mut history = BenchHistory::new();
        for i in 0..3 {
            let config = BenchConfig::quick();
            let mut suite = BenchSuite::new(&format!("run{}", i), config);
            suite.run("op", || {}).unwrap();
            history.add_run(suite);
        }
        let trend = history.trend("op");
        assert_eq!(trend.len(), 3);
    }

    #[test]
    fn test_bench_history_no_regression_with_one_run() {
        let mut history = BenchHistory::new();
        let config = BenchConfig::quick();
        let mut suite = BenchSuite::new("run1", config);
        suite.run("op", || {}).unwrap();
        history.add_run(suite);
        assert!(history.check_regressions(0.1).is_empty());
    }

    #[test]
    fn test_bench_history_save_load() {
        let path =
            std::env::temp_dir().join(format!("evap_bench_hist_{}.json", std::process::id()));
        let mut history = BenchHistory::new();
        let config = BenchConfig::quick();
        let mut suite = BenchSuite::new("saved", config);
        suite.run("op", || {}).unwrap();
        history.add_run(suite);
        history.save(&path).unwrap();

        let loaded = BenchHistory::load(&path).unwrap();
        assert_eq!(loaded.runs.len(), 1);
        assert_eq!(loaded.runs[0].name, "saved");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_run_wallet_benchmarks() {
        let config = BenchConfig::quick();
        let suite = run_wallet_benchmarks(&config).unwrap();
        assert!(suite.results.len() >= 6);
        for r in &suite.results {
            assert!(r.iterations > 0);
            assert!(r.ops_per_sec > 0.0);
        }
    }

    #[test]
    fn test_format_ns() {
        assert_eq!(format_ns(500), "500ns");
        assert_eq!(format_ns(1_500), "1.50us");
        assert_eq!(format_ns(1_500_000), "1.50ms");
        assert_eq!(format_ns(1_500_000_000), "1.50s");
    }

    #[test]
    fn test_compute_stats_single() {
        let mut timings = vec![100u128];
        let stats = compute_stats("single", &mut timings);
        assert_eq!(stats.min_ns, 100);
        assert_eq!(stats.max_ns, 100);
        assert_eq!(stats.mean_ns, 100);
        assert_eq!(stats.median_ns, 100);
    }

    #[test]
    fn test_compute_stats_even_count() {
        let mut timings = vec![10, 20, 30, 40];
        let stats = compute_stats("even", &mut timings);
        assert_eq!(stats.median_ns, 25); // (20+30)/2
        assert_eq!(stats.min_ns, 10);
        assert_eq!(stats.max_ns, 40);
    }

    // ─── Additional coverage tests ────────────────────────────────────────────

    #[test]
    fn test_bench_config_thorough_covers_lines_96_101() {
        let c = BenchConfig::thorough();
        assert_eq!(c.warmup_iterations, 50);
        assert_eq!(c.iterations, 1000);
        assert!(c.validate().is_ok());
    }

    #[test]
    fn test_compute_stats_zero_mean_covers_line_158() {
        // All-zero timings → mean == 0 → ops_per_sec = f64::INFINITY
        let mut timings = vec![0u128; 5];
        let stats = compute_stats("zeros", &mut timings);
        assert_eq!(stats.ops_per_sec, f64::INFINITY);
    }

    #[test]
    fn test_bench_suite_add_covers_lines_194_196() {
        let config = BenchConfig::quick();
        let mut suite = BenchSuite::new("adder", config.clone());
        let dummy_result = run_bench("dummy", &config, || {}).unwrap();
        suite.add(dummy_result);
        assert_eq!(suite.results.len(), 1);
    }

    #[test]
    fn test_compare_zero_mean_covers_line_259() {
        let baseline = BenchResult {
            name: "test".into(),
            iterations: 10,
            total_ns: 1000,
            min_ns: 100,
            max_ns: 100,
            mean_ns: 100,
            median_ns: 100,
            p99_ns: 100,
            ops_per_sec: 1_000_000.0,
        };
        let zero_current = BenchResult { mean_ns: 0, ..baseline.clone() };
        let cmp = compare(&baseline, &zero_current, 0.1);
        assert_eq!(cmp.speedup, f64::INFINITY);
    }

    #[test]
    fn test_get_baseline_empty_history_covers_line_353() {
        let history = BenchHistory::new();
        assert!(history.get_baseline("anything").is_none());
    }

    #[test]
    fn test_check_regressions_with_two_runs_covers_lines_370_392() {
        let mut history = BenchHistory::new();

        // First run (baseline): fast
        let config = BenchConfig::quick();
        let mut baseline_suite = BenchSuite::new("run1", config.clone());
        baseline_suite.run("op", || {}).unwrap();
        // Manually set a specific mean so we can control the regression
        baseline_suite.results[0].mean_ns = 100;
        history.add_run(baseline_suite);

        // Second run: slower (200ns vs 100ns baseline → 0.5× speedup → regression if threshold < 0.5)
        let mut slow_suite = BenchSuite::new("run2", config);
        slow_suite.run("op", || {}).unwrap();
        slow_suite.results[0].mean_ns = 200;
        history.add_run(slow_suite);

        let regressions = history.check_regressions(0.1);
        assert!(!regressions.is_empty());
        assert!(regressions[0].regression);
    }
}
