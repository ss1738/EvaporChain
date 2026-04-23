//! EvaporChain Oracle Library — secure oracle data ingestion with
//! cryptographic signing, multi-source aggregation, and freshness proofs.

pub mod consensus;
pub mod state;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

// ─────────────────────── Oracle Data Types ─────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OracleReport {
    pub source: String,
    pub key: String,
    pub value: OracleValue,
    pub timestamp: u64,
    pub energy: u64,
    pub half_life: u64,
    pub signature: Option<Vec<u8>>,
    pub reporter_id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum OracleValue {
    Numeric(f64),
    Text(String),
    Json(String),
}

impl OracleValue {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            OracleValue::Numeric(v) => Some(*v),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregatedReport {
    pub key: String,
    pub value: f64,
    pub sources: Vec<String>,
    pub report_count: usize,
    pub min: f64,
    pub max: f64,
    pub median: f64,
    pub timestamp: u64,
    pub confidence: f64,
}

// ─────────────────────── Signing ───────────────────────────────────────

impl OracleReport {
    pub fn signable_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(self.source.as_bytes());
        buf.push(b':');
        buf.extend_from_slice(self.key.as_bytes());
        buf.push(b':');
        match &self.value {
            OracleValue::Numeric(v) => buf.extend_from_slice(&v.to_le_bytes()),
            OracleValue::Text(s) => buf.extend_from_slice(s.as_bytes()),
            OracleValue::Json(s) => buf.extend_from_slice(s.as_bytes()),
        }
        buf.extend_from_slice(&self.timestamp.to_le_bytes());
        buf.extend_from_slice(&self.reporter_id.to_le_bytes());
        buf
    }

    pub fn report_hash(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(&self.signable_bytes());
        hasher.finalize().into()
    }
}

// ─────────────────────── Object ID Generation ─────────────────────────

pub fn object_id(source: &str, key: &str) -> [u8; 20] {
    let mut hasher = Sha256::new();
    hasher.update(source.as_bytes());
    hasher.update(b":");
    hasher.update(key.as_bytes());
    let hash = hasher.finalize();
    let mut id = [0u8; 20];
    id.copy_from_slice(&hash[..20]);
    id
}

pub fn object_id_hex(source: &str, key: &str) -> String {
    format!("0x{}", hex::encode(object_id(source, key)))
}

// ─────────────────────── Freshness Validation ─────────────────────────

#[derive(Debug, Clone)]
pub struct FreshnessConfig {
    pub max_age_secs: u64,
    pub min_sources: usize,
    pub max_deviation_pct: f64,
}

impl Default for FreshnessConfig {
    fn default() -> Self {
        Self {
            max_age_secs: 300,
            min_sources: 1,
            max_deviation_pct: 10.0,
        }
    }
}

impl FreshnessConfig {
    pub fn strict() -> Self {
        Self {
            max_age_secs: 60,
            min_sources: 2,
            max_deviation_pct: 5.0,
        }
    }

    pub fn price_feed() -> Self {
        Self {
            max_age_secs: 120,
            min_sources: 2,
            max_deviation_pct: 3.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ValidationError {
    Stale { age_secs: u64, max_age: u64 },
    InsufficientSources { have: usize, need: usize },
    ExcessiveDeviation { deviation_pct: f64, max_pct: f64 },
    NoSignature,
    InvalidSignature,
    FutureTimestamp { timestamp: u64, now: u64 },
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn validate_freshness(report: &OracleReport, config: &FreshnessConfig) -> Result<(), ValidationError> {
    let now = now_secs();

    if report.timestamp > now + 5 {
        return Err(ValidationError::FutureTimestamp {
            timestamp: report.timestamp,
            now,
        });
    }

    let age = now.saturating_sub(report.timestamp);
    if age > config.max_age_secs {
        return Err(ValidationError::Stale {
            age_secs: age,
            max_age: config.max_age_secs,
        });
    }

    Ok(())
}

pub fn validate_report(report: &OracleReport, config: &FreshnessConfig, require_signature: bool) -> Result<(), ValidationError> {
    validate_freshness(report, config)?;

    if require_signature && report.signature.is_none() {
        return Err(ValidationError::NoSignature);
    }

    Ok(())
}

// ─────────────────────── Multi-Source Aggregation ──────────────────────

#[derive(Debug, Default)]
pub struct Aggregator {
    pending: HashMap<String, Vec<OracleReport>>,
    config: HashMap<String, FreshnessConfig>,
}

impl Aggregator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_config(&mut self, key: &str, config: FreshnessConfig) {
        self.config.insert(key.to_string(), config);
    }

    pub fn submit(&mut self, report: OracleReport) -> Result<(), ValidationError> {
        let config = self.config.get(&report.key)
            .cloned()
            .unwrap_or_default();
        validate_freshness(&report, &config)?;

        let reports = self.pending.entry(report.key.clone()).or_default();
        if !reports.iter().any(|r| r.source == report.source && r.reporter_id == report.reporter_id) {
            reports.push(report);
        }
        Ok(())
    }

    pub fn aggregate(&mut self, key: &str) -> Result<AggregatedReport, ValidationError> {
        let config = self.config.get(key).cloned().unwrap_or_default();
        let reports = self.pending.get(key).cloned().unwrap_or_default();

        if reports.len() < config.min_sources {
            return Err(ValidationError::InsufficientSources {
                have: reports.len(),
                need: config.min_sources,
            });
        }

        let mut values: Vec<f64> = reports.iter()
            .filter_map(|r| r.value.as_f64())
            .collect();

        if values.is_empty() {
            return Err(ValidationError::InsufficientSources { have: 0, need: config.min_sources });
        }

        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let min = values[0];
        let max = values[values.len() - 1];
        let median = if values.len() % 2 == 0 {
            (values[values.len() / 2 - 1] + values[values.len() / 2]) / 2.0
        } else {
            values[values.len() / 2]
        };

        if median > 0.0 {
            let deviation_pct = ((max - min) / median) * 100.0;
            if deviation_pct > config.max_deviation_pct {
                return Err(ValidationError::ExcessiveDeviation {
                    deviation_pct,
                    max_pct: config.max_deviation_pct,
                });
            }
        }

        let confidence = if values.len() >= 3 {
            let spread = if median > 0.0 { (max - min) / median } else { 0.0 };
            (1.0 - spread).max(0.0).min(1.0)
        } else if values.len() == 2 {
            0.8
        } else {
            0.5
        };

        let sources: Vec<String> = reports.iter().map(|r| r.source.clone()).collect();
        let timestamp = reports.iter().map(|r| r.timestamp).max().unwrap_or(0);

        self.pending.remove(key);

        Ok(AggregatedReport {
            key: key.to_string(),
            value: median,
            sources,
            report_count: values.len(),
            min,
            max,
            median,
            timestamp,
            confidence,
        })
    }

    pub fn pending_count(&self, key: &str) -> usize {
        self.pending.get(key).map_or(0, |v| v.len())
    }

    pub fn clear(&mut self, key: &str) {
        self.pending.remove(key);
    }

    pub fn clear_all(&mut self) {
        self.pending.clear();
    }
}

// ─────────────────────── Energy/Half-life Presets ──────────────────────

pub struct EnergyPreset {
    pub energy: u64,
    pub half_life: u64,
}

pub mod presets {
    use super::EnergyPreset;

    pub const PRICE_FEED: EnergyPreset = EnergyPreset { energy: 3000, half_life: 60 };
    pub const WEATHER: EnergyPreset = EnergyPreset { energy: 2000, half_life: 300 };
    pub const EARTHQUAKE: EnergyPreset = EnergyPreset { energy: 10000, half_life: 3600 };
    pub const SATELLITE_POSITION: EnergyPreset = EnergyPreset { energy: 5000, half_life: 30 };
    pub const MEMPOOL_STATE: EnergyPreset = EnergyPreset { energy: 4000, half_life: 30 };
    pub const GEOMAGNETIC_STORM: EnergyPreset = EnergyPreset { energy: 50000, half_life: 1800 };
    pub const AIRCRAFT_POSITION: EnergyPreset = EnergyPreset { energy: 2000, half_life: 30 };
}

// ─────────────────────── Tests ────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_report(source: &str, key: &str, value: f64, reporter: u64) -> OracleReport {
        OracleReport {
            source: source.to_string(),
            key: key.to_string(),
            value: OracleValue::Numeric(value),
            timestamp: now_secs(),
            energy: 3000,
            half_life: 60,
            signature: None,
            reporter_id: reporter,
        }
    }

    fn make_report_at(source: &str, key: &str, value: f64, reporter: u64, timestamp: u64) -> OracleReport {
        OracleReport {
            source: source.to_string(),
            key: key.to_string(),
            value: OracleValue::Numeric(value),
            timestamp,
            energy: 3000,
            half_life: 60,
            signature: None,
            reporter_id: reporter,
        }
    }

    // ── Object ID tests ──

    #[test]
    fn test_object_id_deterministic() {
        let id1 = object_id("coingecko", "bitcoin");
        let id2 = object_id("coingecko", "bitcoin");
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_object_id_different_sources() {
        let id1 = object_id("coingecko", "bitcoin");
        let id2 = object_id("binance", "bitcoin");
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_object_id_hex_format() {
        let hex = object_id_hex("test", "key");
        assert!(hex.starts_with("0x"));
        assert_eq!(hex.len(), 42); // 0x + 40 hex chars
    }

    // ── Report hash tests ──

    #[test]
    fn test_report_hash_deterministic() {
        let r = make_report_at("src", "key", 42.0, 1, 1000);
        let h1 = r.report_hash();
        let h2 = r.report_hash();
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_report_hash_differs_on_value() {
        let r1 = make_report_at("src", "key", 42.0, 1, 1000);
        let r2 = make_report_at("src", "key", 43.0, 1, 1000);
        assert_ne!(r1.report_hash(), r2.report_hash());
    }

    #[test]
    fn test_report_hash_differs_on_reporter() {
        let r1 = make_report_at("src", "key", 42.0, 1, 1000);
        let r2 = make_report_at("src", "key", 42.0, 2, 1000);
        assert_ne!(r1.report_hash(), r2.report_hash());
    }

    // ── Freshness tests ──

    #[test]
    fn test_fresh_report_passes() {
        let r = make_report("src", "key", 42.0, 1);
        let config = FreshnessConfig::default();
        assert!(validate_freshness(&r, &config).is_ok());
    }

    #[test]
    fn test_stale_report_rejected() {
        let r = make_report_at("src", "key", 42.0, 1, now_secs() - 600);
        let config = FreshnessConfig { max_age_secs: 300, ..Default::default() };
        match validate_freshness(&r, &config) {
            Err(ValidationError::Stale { .. }) => {},
            other => panic!("expected Stale, got {:?}", other),
        }
    }

    #[test]
    fn test_future_timestamp_rejected() {
        let r = make_report_at("src", "key", 42.0, 1, now_secs() + 100);
        let config = FreshnessConfig::default();
        match validate_freshness(&r, &config) {
            Err(ValidationError::FutureTimestamp { .. }) => {},
            other => panic!("expected FutureTimestamp, got {:?}", other),
        }
    }

    #[test]
    fn test_slight_future_allowed() {
        let r = make_report_at("src", "key", 42.0, 1, now_secs() + 3);
        let config = FreshnessConfig::default();
        assert!(validate_freshness(&r, &config).is_ok());
    }

    // ── Signature validation tests ──

    #[test]
    fn test_unsigned_report_rejected_when_required() {
        let r = make_report("src", "key", 42.0, 1);
        let config = FreshnessConfig::default();
        match validate_report(&r, &config, true) {
            Err(ValidationError::NoSignature) => {},
            other => panic!("expected NoSignature, got {:?}", other),
        }
    }

    #[test]
    fn test_unsigned_report_ok_when_not_required() {
        let r = make_report("src", "key", 42.0, 1);
        let config = FreshnessConfig::default();
        assert!(validate_report(&r, &config, false).is_ok());
    }

    #[test]
    fn test_signed_report_passes() {
        let mut r = make_report("src", "key", 42.0, 1);
        r.signature = Some(vec![1, 2, 3, 4]);
        let config = FreshnessConfig::default();
        assert!(validate_report(&r, &config, true).is_ok());
    }

    // ── Aggregator tests ──

    #[test]
    fn test_single_source_aggregation() {
        let mut agg = Aggregator::new();
        agg.submit(make_report("src1", "btc_usd", 60000.0, 1)).unwrap();
        let result = agg.aggregate("btc_usd").unwrap();
        assert_eq!(result.value, 60000.0);
        assert_eq!(result.report_count, 1);
        assert_eq!(result.confidence, 0.5);
    }

    #[test]
    fn test_two_source_median() {
        let mut agg = Aggregator::new();
        agg.submit(make_report("src1", "btc_usd", 60000.0, 1)).unwrap();
        agg.submit(make_report("src2", "btc_usd", 60200.0, 2)).unwrap();
        let result = agg.aggregate("btc_usd").unwrap();
        assert_eq!(result.median, 60100.0);
        assert_eq!(result.report_count, 2);
        assert_eq!(result.confidence, 0.8);
    }

    #[test]
    fn test_three_source_median() {
        let mut agg = Aggregator::new();
        agg.submit(make_report("src1", "btc_usd", 60000.0, 1)).unwrap();
        agg.submit(make_report("src2", "btc_usd", 60500.0, 2)).unwrap();
        agg.submit(make_report("src3", "btc_usd", 60200.0, 3)).unwrap();
        let result = agg.aggregate("btc_usd").unwrap();
        assert_eq!(result.median, 60200.0);
        assert_eq!(result.report_count, 3);
        assert!(result.confidence > 0.9);
    }

    #[test]
    fn test_aggregation_clears_pending() {
        let mut agg = Aggregator::new();
        agg.submit(make_report("src1", "btc_usd", 60000.0, 1)).unwrap();
        assert_eq!(agg.pending_count("btc_usd"), 1);
        let _ = agg.aggregate("btc_usd").unwrap();
        assert_eq!(agg.pending_count("btc_usd"), 0);
    }

    #[test]
    fn test_insufficient_sources_rejected() {
        let mut agg = Aggregator::new();
        agg.set_config("btc_usd", FreshnessConfig::price_feed());
        agg.submit(make_report("src1", "btc_usd", 60000.0, 1)).unwrap();
        match agg.aggregate("btc_usd") {
            Err(ValidationError::InsufficientSources { have: 1, need: 2 }) => {},
            other => panic!("expected InsufficientSources, got {:?}", other),
        }
    }

    #[test]
    fn test_excessive_deviation_rejected() {
        let mut agg = Aggregator::new();
        agg.set_config("btc_usd", FreshnessConfig {
            max_deviation_pct: 1.0,
            min_sources: 1,
            ..Default::default()
        });
        agg.submit(make_report("src1", "btc_usd", 60000.0, 1)).unwrap();
        agg.submit(make_report("src2", "btc_usd", 65000.0, 2)).unwrap();
        match agg.aggregate("btc_usd") {
            Err(ValidationError::ExcessiveDeviation { .. }) => {},
            other => panic!("expected ExcessiveDeviation, got {:?}", other),
        }
    }

    #[test]
    fn test_dedup_same_source_same_reporter() {
        let mut agg = Aggregator::new();
        agg.submit(make_report("src1", "btc_usd", 60000.0, 1)).unwrap();
        agg.submit(make_report("src1", "btc_usd", 61000.0, 1)).unwrap();
        assert_eq!(agg.pending_count("btc_usd"), 1);
    }

    #[test]
    fn test_same_source_different_reporter_accepted() {
        let mut agg = Aggregator::new();
        agg.submit(make_report("src1", "btc_usd", 60000.0, 1)).unwrap();
        agg.submit(make_report("src1", "btc_usd", 60100.0, 2)).unwrap();
        assert_eq!(agg.pending_count("btc_usd"), 2);
    }

    #[test]
    fn test_stale_report_not_aggregated() {
        let mut agg = Aggregator::new();
        agg.set_config("btc_usd", FreshnessConfig { max_age_secs: 60, ..Default::default() });
        let stale = make_report_at("src1", "btc_usd", 60000.0, 1, now_secs() - 120);
        match agg.submit(stale) {
            Err(ValidationError::Stale { .. }) => {},
            other => panic!("expected Stale, got {:?}", other),
        }
        assert_eq!(agg.pending_count("btc_usd"), 0);
    }

    #[test]
    fn test_clear_removes_pending() {
        let mut agg = Aggregator::new();
        agg.submit(make_report("src1", "btc_usd", 60000.0, 1)).unwrap();
        agg.submit(make_report("src1", "eth_usd", 3000.0, 1)).unwrap();
        agg.clear("btc_usd");
        assert_eq!(agg.pending_count("btc_usd"), 0);
        assert_eq!(agg.pending_count("eth_usd"), 1);
    }

    #[test]
    fn test_clear_all() {
        let mut agg = Aggregator::new();
        agg.submit(make_report("src1", "btc_usd", 60000.0, 1)).unwrap();
        agg.submit(make_report("src1", "eth_usd", 3000.0, 1)).unwrap();
        agg.clear_all();
        assert_eq!(agg.pending_count("btc_usd"), 0);
        assert_eq!(agg.pending_count("eth_usd"), 0);
    }

    // ── OracleValue tests ──

    #[test]
    fn test_oracle_value_numeric() {
        let v = OracleValue::Numeric(42.5);
        assert_eq!(v.as_f64(), Some(42.5));
    }

    #[test]
    fn test_oracle_value_text_no_f64() {
        let v = OracleValue::Text("hello".to_string());
        assert_eq!(v.as_f64(), None);
    }

    #[test]
    fn test_oracle_value_json_no_f64() {
        let v = OracleValue::Json("{\"a\":1}".to_string());
        assert_eq!(v.as_f64(), None);
    }

    // ── Aggregation edge cases ──

    #[test]
    fn test_empty_key_aggregation_fails() {
        let mut agg = Aggregator::new();
        match agg.aggregate("nonexistent") {
            Err(ValidationError::InsufficientSources { have: 0, .. }) => {},
            other => panic!("expected InsufficientSources, got {:?}", other),
        }
    }

    #[test]
    fn test_five_source_outlier_handling() {
        let mut agg = Aggregator::new();
        agg.set_config("btc_usd", FreshnessConfig {
            min_sources: 3,
            max_deviation_pct: 5.0,
            ..Default::default()
        });
        agg.submit(make_report("src1", "btc_usd", 60000.0, 1)).unwrap();
        agg.submit(make_report("src2", "btc_usd", 60100.0, 2)).unwrap();
        agg.submit(make_report("src3", "btc_usd", 60050.0, 3)).unwrap();
        agg.submit(make_report("src4", "btc_usd", 60080.0, 4)).unwrap();
        agg.submit(make_report("src5", "btc_usd", 60020.0, 5)).unwrap();
        let result = agg.aggregate("btc_usd").unwrap();
        assert_eq!(result.median, 60050.0);
        assert_eq!(result.report_count, 5);
        assert!(result.confidence > 0.99);
    }

    #[test]
    fn test_min_max_correct() {
        let mut agg = Aggregator::new();
        agg.set_config("x", FreshnessConfig { max_deviation_pct: 200.0, ..Default::default() });
        agg.submit(make_report("src1", "x", 10.0, 1)).unwrap();
        agg.submit(make_report("src2", "x", 30.0, 2)).unwrap();
        agg.submit(make_report("src3", "x", 20.0, 3)).unwrap();
        let result = agg.aggregate("x").unwrap();
        assert_eq!(result.min, 10.0);
        assert_eq!(result.max, 30.0);
    }

    // ── Energy presets ──

    #[test]
    fn test_presets_reasonable() {
        assert!(presets::EARTHQUAKE.energy > presets::WEATHER.energy);
        assert!(presets::EARTHQUAKE.half_life > presets::PRICE_FEED.half_life);
        assert!(presets::SATELLITE_POSITION.half_life <= presets::WEATHER.half_life);
        assert!(presets::GEOMAGNETIC_STORM.energy > presets::EARTHQUAKE.energy);
    }

    // ── Serialization roundtrip ──

    #[test]
    fn test_report_serialization_roundtrip() {
        let r = make_report("coingecko", "btc_usd", 60000.0, 1);
        let json = serde_json::to_string(&r).unwrap();
        let r2: OracleReport = serde_json::from_str(&json).unwrap();
        assert_eq!(r, r2);
    }

    #[test]
    fn test_aggregated_report_serialization() {
        let mut agg = Aggregator::new();
        agg.submit(make_report("src1", "btc_usd", 60000.0, 1)).unwrap();
        let result = agg.aggregate("btc_usd").unwrap();
        let json = serde_json::to_string(&result).unwrap();
        let r2: AggregatedReport = serde_json::from_str(&json).unwrap();
        assert_eq!(r2.value, 60000.0);
    }
}
