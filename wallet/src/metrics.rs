// wallet/src/metrics.rs — Prometheus-style metrics for wallet telemetry
//
// Counters (monotonic), gauges (up/down), histograms (distribution buckets).
// Thread-safe via Arc<Mutex<>>, exportable as Prometheus text format or JSON.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MetricsError {
    #[error("metric not found: {0}")]
    NotFound(String),
    #[error("metric type mismatch: {0} is a {1}, not a {2}")]
    TypeMismatch(String, String, String),
    #[error("invalid bucket boundary: {0}")]
    InvalidBucket(String),
}

// ── Metric types ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Counter {
    pub name: String,
    pub help: String,
    pub value: f64,
    pub labels: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Gauge {
    pub name: String,
    pub help: String,
    pub value: f64,
    pub labels: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Histogram {
    pub name: String,
    pub help: String,
    pub buckets: Vec<f64>,          // upper bounds
    pub counts: Vec<u64>,           // per-bucket counts
    pub sum: f64,
    pub count: u64,
    pub labels: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Metric {
    Counter(Counter),
    Gauge(Gauge),
    Histogram(Histogram),
}

impl Metric {
    pub fn name(&self) -> &str {
        match self {
            Metric::Counter(c) => &c.name,
            Metric::Gauge(g) => &g.name,
            Metric::Histogram(h) => &h.name,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Metric::Counter(_) => "counter",
            Metric::Gauge(_) => "gauge",
            Metric::Histogram(_) => "histogram",
        }
    }
}

// ── Registry ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MetricsRegistry {
    pub metrics: BTreeMap<String, Metric>,
}

impl MetricsRegistry {
    pub fn new() -> Self {
        Self {
            metrics: BTreeMap::new(),
        }
    }

    // ── Counter ops ───────────────────────────────────────────

    pub fn register_counter(
        &mut self,
        name: &str,
        help: &str,
        labels: BTreeMap<String, String>,
    ) {
        self.metrics.insert(
            name.to_string(),
            Metric::Counter(Counter {
                name: name.to_string(),
                help: help.to_string(),
                value: 0.0,
                labels,
            }),
        );
    }

    pub fn counter_inc(&mut self, name: &str) -> Result<(), MetricsError> {
        self.counter_add(name, 1.0)
    }

    pub fn counter_add(&mut self, name: &str, v: f64) -> Result<(), MetricsError> {
        match self.metrics.get_mut(name) {
            Some(Metric::Counter(c)) => {
                c.value += v;
                Ok(())
            }
            Some(m) => Err(MetricsError::TypeMismatch(
                name.into(),
                m.type_name().into(),
                "counter".into(),
            )),
            None => Err(MetricsError::NotFound(name.into())),
        }
    }

    pub fn counter_get(&self, name: &str) -> Result<f64, MetricsError> {
        match self.metrics.get(name) {
            Some(Metric::Counter(c)) => Ok(c.value),
            Some(m) => Err(MetricsError::TypeMismatch(
                name.into(),
                m.type_name().into(),
                "counter".into(),
            )),
            None => Err(MetricsError::NotFound(name.into())),
        }
    }

    // ── Gauge ops ─────────────────────────────────────────────

    pub fn register_gauge(
        &mut self,
        name: &str,
        help: &str,
        labels: BTreeMap<String, String>,
    ) {
        self.metrics.insert(
            name.to_string(),
            Metric::Gauge(Gauge {
                name: name.to_string(),
                help: help.to_string(),
                value: 0.0,
                labels,
            }),
        );
    }

    pub fn gauge_set(&mut self, name: &str, v: f64) -> Result<(), MetricsError> {
        match self.metrics.get_mut(name) {
            Some(Metric::Gauge(g)) => {
                g.value = v;
                Ok(())
            }
            Some(m) => Err(MetricsError::TypeMismatch(
                name.into(),
                m.type_name().into(),
                "gauge".into(),
            )),
            None => Err(MetricsError::NotFound(name.into())),
        }
    }

    pub fn gauge_inc(&mut self, name: &str) -> Result<(), MetricsError> {
        self.gauge_add(name, 1.0)
    }

    pub fn gauge_dec(&mut self, name: &str) -> Result<(), MetricsError> {
        self.gauge_add(name, -1.0)
    }

    pub fn gauge_add(&mut self, name: &str, v: f64) -> Result<(), MetricsError> {
        match self.metrics.get_mut(name) {
            Some(Metric::Gauge(g)) => {
                g.value += v;
                Ok(())
            }
            Some(m) => Err(MetricsError::TypeMismatch(
                name.into(),
                m.type_name().into(),
                "gauge".into(),
            )),
            None => Err(MetricsError::NotFound(name.into())),
        }
    }

    pub fn gauge_get(&self, name: &str) -> Result<f64, MetricsError> {
        match self.metrics.get(name) {
            Some(Metric::Gauge(g)) => Ok(g.value),
            Some(m) => Err(MetricsError::TypeMismatch(
                name.into(),
                m.type_name().into(),
                "gauge".into(),
            )),
            None => Err(MetricsError::NotFound(name.into())),
        }
    }

    // ── Histogram ops ─────────────────────────────────────────

    pub fn register_histogram(
        &mut self,
        name: &str,
        help: &str,
        buckets: Vec<f64>,
        labels: BTreeMap<String, String>,
    ) -> Result<(), MetricsError> {
        if buckets.is_empty() {
            return Err(MetricsError::InvalidBucket("buckets cannot be empty".into()));
        }
        for w in buckets.windows(2) {
            if w[0] >= w[1] {
                return Err(MetricsError::InvalidBucket(format!(
                    "buckets must be strictly ascending: {} >= {}",
                    w[0], w[1]
                )));
            }
        }
        let len = buckets.len();
        self.metrics.insert(
            name.to_string(),
            Metric::Histogram(Histogram {
                name: name.to_string(),
                help: help.to_string(),
                buckets,
                counts: vec![0; len],
                sum: 0.0,
                count: 0,
                labels,
            }),
        );
        Ok(())
    }

    pub fn histogram_observe(&mut self, name: &str, v: f64) -> Result<(), MetricsError> {
        match self.metrics.get_mut(name) {
            Some(Metric::Histogram(h)) => {
                h.sum += v;
                h.count += 1;
                for (i, &bound) in h.buckets.iter().enumerate() {
                    if v <= bound {
                        h.counts[i] += 1;
                    }
                }
                Ok(())
            }
            Some(m) => Err(MetricsError::TypeMismatch(
                name.into(),
                m.type_name().into(),
                "histogram".into(),
            )),
            None => Err(MetricsError::NotFound(name.into())),
        }
    }

    pub fn histogram_get(&self, name: &str) -> Result<&Histogram, MetricsError> {
        match self.metrics.get(name) {
            Some(Metric::Histogram(h)) => Ok(h),
            Some(m) => Err(MetricsError::TypeMismatch(
                name.into(),
                m.type_name().into(),
                "histogram".into(),
            )),
            None => Err(MetricsError::NotFound(name.into())),
        }
    }

    // ── Queries ───────────────────────────────────────────────

    pub fn list(&self) -> Vec<&Metric> {
        self.metrics.values().collect()
    }

    pub fn get(&self, name: &str) -> Option<&Metric> {
        self.metrics.get(name)
    }

    pub fn remove(&mut self, name: &str) -> bool {
        self.metrics.remove(name).is_some()
    }

    pub fn reset(&mut self) {
        for m in self.metrics.values_mut() {
            match m {
                Metric::Counter(c) => c.value = 0.0,
                Metric::Gauge(g) => g.value = 0.0,
                Metric::Histogram(h) => {
                    h.sum = 0.0;
                    h.count = 0;
                    for c in h.counts.iter_mut() {
                        *c = 0;
                    }
                }
            }
        }
    }

    // ── Export ─────────────────────────────────────────────────

    /// Prometheus text exposition format
    pub fn to_prometheus(&self) -> String {
        let mut out = String::new();
        for m in self.metrics.values() {
            match m {
                Metric::Counter(c) => {
                    out.push_str(&format!("# HELP {} {}\n", c.name, c.help));
                    out.push_str(&format!("# TYPE {} counter\n", c.name));
                    out.push_str(&format!(
                        "{}{} {}\n",
                        c.name,
                        format_labels(&c.labels),
                        format_value(c.value)
                    ));
                }
                Metric::Gauge(g) => {
                    out.push_str(&format!("# HELP {} {}\n", g.name, g.help));
                    out.push_str(&format!("# TYPE {} gauge\n", g.name));
                    out.push_str(&format!(
                        "{}{} {}\n",
                        g.name,
                        format_labels(&g.labels),
                        format_value(g.value)
                    ));
                }
                Metric::Histogram(h) => {
                    out.push_str(&format!("# HELP {} {}\n", h.name, h.help));
                    out.push_str(&format!("# TYPE {} histogram\n", h.name));
                    let lbl = format_labels(&h.labels);
                    for (i, &bound) in h.buckets.iter().enumerate() {
                        let le = if bound == f64::INFINITY {
                            "+Inf".to_string()
                        } else {
                            format_value(bound)
                        };
                        let mut bucket_labels = h.labels.clone();
                        bucket_labels.insert("le".to_string(), le);
                        out.push_str(&format!(
                            "{}_bucket{} {}\n",
                            h.name,
                            format_labels(&bucket_labels),
                            h.counts[i]
                        ));
                    }
                    out.push_str(&format!(
                        "{}_sum{} {}\n",
                        h.name,
                        lbl,
                        format_value(h.sum)
                    ));
                    out.push_str(&format!("{}_count{} {}\n", h.name, lbl, h.count));
                }
            }
        }
        out
    }

    /// JSON export of all metrics
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(&self.metrics).unwrap_or_default()
    }

    // ── Convenience: default wallet metrics ───────────────────

    pub fn register_wallet_defaults(&mut self) {
        self.register_counter(
            "wallet_tx_total",
            "Total transactions sent",
            BTreeMap::new(),
        );
        self.register_counter(
            "wallet_tx_failed_total",
            "Total failed transactions",
            BTreeMap::new(),
        );
        self.register_counter(
            "wallet_refresh_total",
            "Total energy refreshes",
            BTreeMap::new(),
        );
        self.register_gauge(
            "wallet_balance",
            "Current wallet balance",
            BTreeMap::new(),
        );
        self.register_gauge(
            "wallet_energy_lowest",
            "Lowest energy level among objects",
            BTreeMap::new(),
        );
        self.register_gauge(
            "wallet_objects_count",
            "Number of objects owned",
            BTreeMap::new(),
        );
        let _ = self.register_histogram(
            "wallet_tx_latency_ms",
            "Transaction confirmation latency in ms",
            vec![100.0, 250.0, 500.0, 1000.0, 2500.0, 5000.0, 10000.0],
            BTreeMap::new(),
        );
        let _ = self.register_histogram(
            "wallet_gas_used",
            "Gas used per transaction",
            vec![1000.0, 5000.0, 10000.0, 50000.0, 100000.0],
            BTreeMap::new(),
        );
    }
}

impl fmt::Display for MetricsRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_prometheus())
    }
}

// ── Helpers ───────────────────────────────────────────────────

fn format_labels(labels: &BTreeMap<String, String>) -> String {
    if labels.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = labels
        .iter()
        .map(|(k, v)| format!("{}=\"{}\"", k, v))
        .collect();
    format!("{{{}}}", parts.join(","))
}

fn format_value(v: f64) -> String {
    if v == v.floor() && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{}", v)
    }
}

// ── Default bucket sets ───────────────────────────────────────

pub fn default_latency_buckets() -> Vec<f64> {
    vec![10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0, 2500.0, 5000.0, 10000.0]
}

pub fn default_size_buckets() -> Vec<f64> {
    vec![64.0, 256.0, 1024.0, 4096.0, 16384.0, 65536.0]
}

pub fn exponential_buckets(start: f64, factor: f64, count: usize) -> Result<Vec<f64>, MetricsError> {
    if start <= 0.0 {
        return Err(MetricsError::InvalidBucket("start must be positive".into()));
    }
    if factor <= 1.0 {
        return Err(MetricsError::InvalidBucket("factor must be > 1".into()));
    }
    if count == 0 {
        return Err(MetricsError::InvalidBucket("count must be > 0".into()));
    }
    let mut buckets = Vec::with_capacity(count);
    let mut v = start;
    for _ in 0..count {
        buckets.push(v);
        v *= factor;
    }
    Ok(buckets)
}

pub fn linear_buckets(start: f64, width: f64, count: usize) -> Result<Vec<f64>, MetricsError> {
    if width <= 0.0 {
        return Err(MetricsError::InvalidBucket("width must be positive".into()));
    }
    if count == 0 {
        return Err(MetricsError::InvalidBucket("count must be > 0".into()));
    }
    let mut buckets = Vec::with_capacity(count);
    let mut v = start;
    for _ in 0..count {
        buckets.push(v);
        v += width;
    }
    Ok(buckets)
}

// ── Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn reg() -> MetricsRegistry {
        MetricsRegistry::new()
    }

    #[test]
    fn test_counter_basic() {
        let mut r = reg();
        r.register_counter("tx_total", "total txs", BTreeMap::new());
        r.counter_inc("tx_total").unwrap();
        r.counter_inc("tx_total").unwrap();
        r.counter_add("tx_total", 3.0).unwrap();
        assert_eq!(r.counter_get("tx_total").unwrap(), 5.0);
    }

    #[test]
    fn test_counter_not_found() {
        let mut r = reg();
        assert!(r.counter_inc("nope").is_err());
    }

    #[test]
    fn test_counter_type_mismatch() {
        let mut r = reg();
        r.register_gauge("g", "a gauge", BTreeMap::new());
        assert!(r.counter_inc("g").is_err());
    }

    #[test]
    fn test_gauge_set_and_get() {
        let mut r = reg();
        r.register_gauge("balance", "wallet balance", BTreeMap::new());
        r.gauge_set("balance", 1000.0).unwrap();
        assert_eq!(r.gauge_get("balance").unwrap(), 1000.0);
    }

    #[test]
    fn test_gauge_inc_dec() {
        let mut r = reg();
        r.register_gauge("active", "active sessions", BTreeMap::new());
        r.gauge_inc("active").unwrap();
        r.gauge_inc("active").unwrap();
        r.gauge_dec("active").unwrap();
        assert_eq!(r.gauge_get("active").unwrap(), 1.0);
    }

    #[test]
    fn test_gauge_add() {
        let mut r = reg();
        r.register_gauge("g", "gauge", BTreeMap::new());
        r.gauge_add("g", 5.5).unwrap();
        r.gauge_add("g", -2.0).unwrap();
        assert_eq!(r.gauge_get("g").unwrap(), 3.5);
    }

    #[test]
    fn test_histogram_observe() {
        let mut r = reg();
        r.register_histogram(
            "latency",
            "request latency",
            vec![10.0, 50.0, 100.0, 500.0],
            BTreeMap::new(),
        )
        .unwrap();
        r.histogram_observe("latency", 5.0).unwrap();
        r.histogram_observe("latency", 30.0).unwrap();
        r.histogram_observe("latency", 75.0).unwrap();
        r.histogram_observe("latency", 200.0).unwrap();
        let h = r.histogram_get("latency").unwrap();
        assert_eq!(h.count, 4);
        assert_eq!(h.sum, 310.0);
        // Cumulative: 5<=10: +1, 30<=50: +1, 75<=100: +1, 200<=500: +1
        // Each value also fits in all higher buckets
        assert_eq!(h.counts, vec![1, 2, 3, 4]);
    }

    #[test]
    fn test_histogram_cumulative_buckets() {
        let mut r = reg();
        r.register_histogram("h", "test", vec![10.0, 100.0], BTreeMap::new())
            .unwrap();
        // value 5 fits in both buckets (<=10 and <=100)
        r.histogram_observe("h", 5.0).unwrap();
        let h = r.histogram_get("h").unwrap();
        assert_eq!(h.counts, vec![1, 1]);
    }

    #[test]
    fn test_histogram_empty_buckets_rejected() {
        let mut r = reg();
        let res = r.register_histogram("h", "bad", vec![], BTreeMap::new());
        assert!(res.is_err());
    }

    #[test]
    fn test_histogram_unordered_buckets_rejected() {
        let mut r = reg();
        let res = r.register_histogram("h", "bad", vec![100.0, 50.0], BTreeMap::new());
        assert!(res.is_err());
    }

    #[test]
    fn test_list_metrics() {
        let mut r = reg();
        r.register_counter("a", "a", BTreeMap::new());
        r.register_gauge("b", "b", BTreeMap::new());
        assert_eq!(r.list().len(), 2);
    }

    #[test]
    fn test_remove_metric() {
        let mut r = reg();
        r.register_counter("x", "x", BTreeMap::new());
        assert!(r.remove("x"));
        assert!(!r.remove("x"));
        assert!(r.get("x").is_none());
    }

    #[test]
    fn test_reset() {
        let mut r = reg();
        r.register_counter("c", "c", BTreeMap::new());
        r.register_gauge("g", "g", BTreeMap::new());
        r.counter_add("c", 10.0).unwrap();
        r.gauge_set("g", 42.0).unwrap();
        r.reset();
        assert_eq!(r.counter_get("c").unwrap(), 0.0);
        assert_eq!(r.gauge_get("g").unwrap(), 0.0);
    }

    #[test]
    fn test_prometheus_export_counter() {
        let mut r = reg();
        r.register_counter("tx", "transactions", BTreeMap::new());
        r.counter_add("tx", 7.0).unwrap();
        let prom = r.to_prometheus();
        assert!(prom.contains("# HELP tx transactions"));
        assert!(prom.contains("# TYPE tx counter"));
        assert!(prom.contains("tx 7"));
    }

    #[test]
    fn test_prometheus_export_with_labels() {
        let mut r = reg();
        let mut labels = BTreeMap::new();
        labels.insert("chain".to_string(), "evaporchain".to_string());
        r.register_gauge("bal", "balance", labels);
        r.gauge_set("bal", 100.0).unwrap();
        let prom = r.to_prometheus();
        assert!(prom.contains("bal{chain=\"evaporchain\"} 100"));
    }

    #[test]
    fn test_json_export() {
        let mut r = reg();
        r.register_counter("c", "counter", BTreeMap::new());
        r.counter_inc("c").unwrap();
        let json = r.to_json();
        assert!(json.contains("\"c\""));
        assert!(json.contains("counter"));
    }

    #[test]
    fn test_wallet_defaults() {
        let mut r = reg();
        r.register_wallet_defaults();
        assert!(r.get("wallet_tx_total").is_some());
        assert!(r.get("wallet_balance").is_some());
        assert!(r.get("wallet_tx_latency_ms").is_some());
        assert!(r.get("wallet_gas_used").is_some());
        assert_eq!(r.list().len(), 8);
    }

    #[test]
    fn test_exponential_buckets() {
        let b = exponential_buckets(1.0, 2.0, 5).unwrap();
        assert_eq!(b, vec![1.0, 2.0, 4.0, 8.0, 16.0]);
    }

    #[test]
    fn test_exponential_buckets_bad_start() {
        assert!(exponential_buckets(0.0, 2.0, 5).is_err());
    }

    #[test]
    fn test_exponential_buckets_bad_factor() {
        assert!(exponential_buckets(1.0, 0.5, 5).is_err());
    }

    #[test]
    fn test_linear_buckets() {
        let b = linear_buckets(10.0, 5.0, 4).unwrap();
        assert_eq!(b, vec![10.0, 15.0, 20.0, 25.0]);
    }

    #[test]
    fn test_linear_buckets_bad_width() {
        assert!(linear_buckets(10.0, 0.0, 4).is_err());
    }

    #[test]
    fn test_display_trait() {
        let mut r = reg();
        r.register_counter("d", "display test", BTreeMap::new());
        let s = format!("{}", r);
        assert!(s.contains("# TYPE d counter"));
    }

    #[test]
    fn test_histogram_type_mismatch() {
        let mut r = reg();
        r.register_counter("c", "counter", BTreeMap::new());
        assert!(r.histogram_observe("c", 1.0).is_err());
        assert!(r.histogram_get("c").is_err());
    }

    #[test]
    fn test_gauge_type_mismatch() {
        let mut r = reg();
        r.register_counter("c", "counter", BTreeMap::new());
        assert!(r.gauge_set("c", 1.0).is_err());
        assert!(r.gauge_get("c").is_err());
    }
}
