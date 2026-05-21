//! Mempool monitoring and gas oracle system.
//!
//! Tracks pending transactions in the mempool, detects potential front-running
//! attacks, and provides gas price recommendations based on observed fee data.
//!
//! Features:
//! - Real-time pending transaction tracking
//! - Front-run detection with risk classification
//! - Gas oracle with percentile-based fee recommendations
//! - Congestion level monitoring
//! - Fee histogram generation

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum MempoolError {
    #[error("transaction already exists: {0}")]
    AlreadyExists(String),
    #[error("transaction not found: {0}")]
    NotFound(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Parse(#[from] serde_json::Error),
}

// ──────────────────────────── Enums ──────────────────────────────────────

/// Transaction priority level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TxPriority {
    Urgent,
    High,
    Medium,
    Low,
}

/// Front-run risk classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FrontRunRisk {
    None,
    Low,
    Medium,
    High,
    Critical,
}

/// Network congestion level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CongestionLevel {
    Empty,
    Low,
    Normal,
    High,
    Critical,
}

// ──────────────────────────── Structs ──────────────────────────────────

/// A pending transaction in the mempool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingTx {
    pub tx_hash: String,
    pub sender: String,
    pub receiver: String,
    pub amount: u64,
    pub fee: u64,
    pub nonce: u64,
    pub tx_type: String,
    pub first_seen: String,
    pub priority: TxPriority,
}

impl PendingTx {
    /// Create a new pending transaction. Priority is auto-computed from fee.
    pub fn new(
        tx_hash: String,
        sender: String,
        receiver: String,
        amount: u64,
        fee: u64,
        nonce: u64,
        tx_type: String,
    ) -> Self {
        let priority = if fee >= 10_000 {
            TxPriority::Urgent
        } else if fee >= 5_000 {
            TxPriority::High
        } else if fee >= 1_000 {
            TxPriority::Medium
        } else {
            TxPriority::Low
        };

        Self {
            tx_hash,
            sender,
            receiver,
            amount,
            fee,
            nonce,
            tx_type,
            first_seen: chrono::Utc::now().to_rfc3339(),
            priority,
        }
    }

    /// Seconds since this transaction was first seen.
    pub fn age_secs(&self) -> u64 {
        let seen = chrono::DateTime::parse_from_rfc3339(&self.first_seen).unwrap_or_else(|_| {
            chrono::DateTime::parse_from_rfc3339("1970-01-01T00:00:00+00:00").unwrap()
        });
        let now = chrono::Utc::now();
        let diff = now.signed_duration_since(seen);
        diff.num_seconds().max(0) as u64
    }

    /// Estimated fee per byte (assumes ~250 byte tx size). Minimum 1.
    pub fn fee_per_byte(&self) -> u64 {
        (self.fee / 250).max(1)
    }
}

/// A single gas fee sample.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GasSample {
    pub fee: u64,
    pub included: bool,
    pub recorded_at: String,
}

/// Gas price oracle based on observed fee samples.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GasOracle {
    pub samples: Vec<GasSample>,
    pub last_updated: Option<String>,
}

impl GasOracle {
    /// Create a new empty gas oracle.
    pub fn new() -> Self {
        Self {
            samples: Vec::new(),
            last_updated: None,
        }
    }

    /// Record a fee sample.
    pub fn record(&mut self, fee: u64, included_in_block: bool) {
        self.samples.push(GasSample {
            fee,
            included: included_in_block,
            recorded_at: chrono::Utc::now().to_rfc3339(),
        });
        if self.samples.len() > 1000 {
            self.samples.remove(0);
        }
        self.last_updated = Some(chrono::Utc::now().to_rfc3339());
    }

    /// Recommend a fee for the given priority level.
    /// Urgent=p95, High=p75, Medium=p50, Low=p25.
    /// Returns 100 if no samples.
    pub fn recommend(&self, priority: &TxPriority) -> u64 {
        if self.samples.is_empty() {
            return 100;
        }
        match priority {
            TxPriority::Urgent => self.percentile(95.0),
            TxPriority::High => self.percentile(75.0),
            TxPriority::Medium => self.percentile(50.0),
            TxPriority::Low => self.percentile(25.0),
        }
    }

    /// Compute the p-th percentile of all sample fees.
    pub fn percentile(&self, p: f64) -> u64 {
        if self.samples.is_empty() {
            return 0;
        }
        let mut fees: Vec<u64> = self.samples.iter().map(|s| s.fee).collect();
        fees.sort();
        let idx = ((p / 100.0) * (fees.len() as f64 - 1.0)).round() as usize;
        let idx = idx.min(fees.len() - 1);
        fees[idx]
    }

    /// Average fee across all samples.
    pub fn average_fee(&self) -> u64 {
        if self.samples.is_empty() {
            return 0;
        }
        let total: u64 = self.samples.iter().map(|s| s.fee).sum();
        total / self.samples.len() as u64
    }

    /// Number of samples stored.
    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }
}

impl Default for GasOracle {
    fn default() -> Self {
        Self::new()
    }
}

/// A front-run detection alert.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrontRunAlert {
    pub victim_hash: String,
    pub attacker_hash: String,
    pub risk: FrontRunRisk,
    pub detected_at: String,
    pub details: String,
}

/// Aggregate mempool statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MempoolStats {
    pub pending_count: usize,
    pub total_fees: u64,
    pub avg_fee: u64,
    pub removed_count: u64,
    pub alert_count: usize,
    pub congestion: CongestionLevel,
}

// ──────────────────────────── MempoolMonitor ──────────────────────────

/// Main mempool monitor: tracks pending transactions, detects front-running,
/// and provides gas price recommendations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MempoolMonitor {
    pub pending: HashMap<String, PendingTx>,
    pub oracle: GasOracle,
    pub alerts: Vec<FrontRunAlert>,
    pub max_alerts: usize,
    pub removed_count: u64,
}

impl Default for MempoolMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl MempoolMonitor {
    /// Create a new mempool monitor.
    pub fn new() -> Self {
        Self {
            pending: HashMap::new(),
            oracle: GasOracle::new(),
            alerts: Vec::new(),
            max_alerts: 200,
            removed_count: 0,
        }
    }

    /// Add a pending transaction. Fails if the hash already exists.
    pub fn add_tx(&mut self, tx: PendingTx) -> Result<(), MempoolError> {
        if self.pending.contains_key(&tx.tx_hash) {
            return Err(MempoolError::AlreadyExists(tx.tx_hash));
        }
        self.pending.insert(tx.tx_hash.clone(), tx);
        Ok(())
    }

    /// Remove a pending transaction by hash. Increments removed_count.
    pub fn remove_tx(&mut self, tx_hash: &str) -> Result<PendingTx, MempoolError> {
        match self.pending.remove(tx_hash) {
            Some(tx) => {
                self.removed_count += 1;
                Ok(tx)
            }
            None => Err(MempoolError::NotFound(tx_hash.to_string())),
        }
    }

    /// Get a reference to a pending transaction.
    pub fn get_tx(&self, tx_hash: &str) -> Option<&PendingTx> {
        self.pending.get(tx_hash)
    }

    /// Number of currently pending transactions.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// All pending transactions from a given sender.
    pub fn pending_by_sender(&self, sender: &str) -> Vec<&PendingTx> {
        self.pending
            .values()
            .filter(|tx| tx.sender == sender)
            .collect()
    }

    /// All pending transactions with a given priority.
    pub fn pending_by_priority(&self, priority: &TxPriority) -> Vec<&PendingTx> {
        self.pending
            .values()
            .filter(|tx| tx.priority == *priority)
            .collect()
    }

    /// Pending transaction with the highest fee.
    pub fn highest_fee(&self) -> Option<&PendingTx> {
        self.pending.values().max_by_key(|tx| tx.fee)
    }

    /// Pending transaction with the lowest fee.
    pub fn lowest_fee(&self) -> Option<&PendingTx> {
        self.pending.values().min_by_key(|tx| tx.fee)
    }

    /// Detect a potential front-run attack between two pending transactions.
    /// Both must be in the mempool. Returns the risk level.
    pub fn detect_front_run(
        &mut self,
        victim_hash: &str,
        attacker_hash: &str,
    ) -> Result<FrontRunRisk, MempoolError> {
        let victim = self
            .pending
            .get(victim_hash)
            .ok_or_else(|| MempoolError::NotFound(victim_hash.to_string()))?
            .clone();
        let attacker = self
            .pending
            .get(attacker_hash)
            .ok_or_else(|| MempoolError::NotFound(attacker_hash.to_string()))?
            .clone();

        let risk = if attacker.receiver == victim.receiver && attacker.fee > victim.fee {
            FrontRunRisk::High
        } else if attacker.tx_type == victim.tx_type && attacker.fee > victim.fee {
            FrontRunRisk::Medium
        } else {
            FrontRunRisk::Low
        };

        let details = format!(
            "Victim fee={}, Attacker fee={}, same_receiver={}, same_type={}",
            victim.fee,
            attacker.fee,
            attacker.receiver == victim.receiver,
            attacker.tx_type == victim.tx_type,
        );

        let alert = FrontRunAlert {
            victim_hash: victim_hash.to_string(),
            attacker_hash: attacker_hash.to_string(),
            risk,
            detected_at: chrono::Utc::now().to_rfc3339(),
            details,
        };

        self.alerts.push(alert);
        if self.alerts.len() > self.max_alerts {
            self.alerts.remove(0);
        }

        Ok(risk)
    }

    /// Current congestion level based on pending transaction count.
    pub fn congestion(&self) -> CongestionLevel {
        let count = self.pending.len();
        if count == 0 {
            CongestionLevel::Empty
        } else if count < 10 {
            CongestionLevel::Low
        } else if count < 50 {
            CongestionLevel::Normal
        } else if count < 200 {
            CongestionLevel::High
        } else {
            CongestionLevel::Critical
        }
    }

    /// Recommend a fee for the given priority. Delegates to the gas oracle.
    pub fn recommend_fee(&self, priority: &TxPriority) -> u64 {
        self.oracle.recommend(priority)
    }

    /// Record that a transaction with the given fee was included in a block.
    pub fn record_inclusion(&mut self, fee: u64) {
        self.oracle.record(fee, true);
    }

    /// Aggregate mempool statistics.
    pub fn stats(&self) -> MempoolStats {
        let total_fees: u64 = self.pending.values().map(|tx| tx.fee).sum();
        let avg_fee = if self.pending.is_empty() {
            0
        } else {
            total_fees / self.pending.len() as u64
        };

        MempoolStats {
            pending_count: self.pending.len(),
            total_fees,
            avg_fee,
            removed_count: self.removed_count,
            alert_count: self.alerts.len(),
            congestion: self.congestion(),
        }
    }

    /// Clear all pending transactions. Returns the number removed.
    pub fn clear_pending(&mut self) -> usize {
        let count = self.pending.len();
        self.pending.clear();
        count
    }

    /// Transactions older than the given number of seconds.
    pub fn stale_txs(&self, age_secs: u64) -> Vec<&PendingTx> {
        self.pending
            .values()
            .filter(|tx| tx.age_secs() > age_secs)
            .collect()
    }

    /// Fee histogram: for each bucket threshold, count transactions with fee <= that threshold.
    pub fn fee_histogram(&self, buckets: &[u64]) -> Vec<(u64, usize)> {
        buckets
            .iter()
            .map(|&threshold| {
                let count = self
                    .pending
                    .values()
                    .filter(|tx| tx.fee <= threshold)
                    .count();
                (threshold, count)
            })
            .collect()
    }

    /// Save the monitor state to a JSON file.
    pub fn save(&self, path: &Path) -> Result<(), MempoolError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Load the monitor state from a JSON file.
    pub fn load(path: &Path) -> Result<Self, MempoolError> {
        let data = std::fs::read_to_string(path)?;
        let monitor: Self = serde_json::from_str(&data)?;
        Ok(monitor)
    }

    /// Load from file or return a default instance if the file cannot be read.
    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ──────────────────────────── Tests ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tx(hash: &str, sender: &str, receiver: &str, fee: u64, tx_type: &str) -> PendingTx {
        PendingTx::new(
            hash.to_string(),
            sender.to_string(),
            receiver.to_string(),
            1000,
            fee,
            1,
            tx_type.to_string(),
        )
    }

    fn test_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("mempool_test_{}_{}", name, std::process::id()))
    }

    #[test]
    fn test_add_and_get_tx() {
        let mut monitor = MempoolMonitor::new();
        let tx = make_tx("hash1", "alice", "bob", 5000, "transfer");
        monitor.add_tx(tx).unwrap();

        let retrieved = monitor.get_tx("hash1").unwrap();
        assert_eq!(retrieved.tx_hash, "hash1");
        assert_eq!(retrieved.sender, "alice");
        assert_eq!(retrieved.receiver, "bob");
        assert_eq!(retrieved.fee, 5000);
        assert_eq!(retrieved.priority, TxPriority::High);
    }

    #[test]
    fn test_add_duplicate_rejected() {
        let mut monitor = MempoolMonitor::new();
        let tx1 = make_tx("hash1", "alice", "bob", 5000, "transfer");
        let tx2 = make_tx("hash1", "charlie", "dave", 3000, "transfer");
        monitor.add_tx(tx1).unwrap();

        let result = monitor.add_tx(tx2);
        assert!(result.is_err());
        match result.unwrap_err() {
            MempoolError::AlreadyExists(h) => assert_eq!(h, "hash1"),
            other => panic!("expected AlreadyExists, got: {:?}", other),
        }
    }

    #[test]
    fn test_remove_tx() {
        let mut monitor = MempoolMonitor::new();
        let tx = make_tx("hash1", "alice", "bob", 5000, "transfer");
        monitor.add_tx(tx).unwrap();

        let removed = monitor.remove_tx("hash1").unwrap();
        assert_eq!(removed.tx_hash, "hash1");
        assert_eq!(monitor.pending_count(), 0);
        assert_eq!(monitor.removed_count, 1);
    }

    #[test]
    fn test_remove_not_found() {
        let mut monitor = MempoolMonitor::new();
        let result = monitor.remove_tx("nonexistent");
        assert!(result.is_err());
        match result.unwrap_err() {
            MempoolError::NotFound(h) => assert_eq!(h, "nonexistent"),
            other => panic!("expected NotFound, got: {:?}", other),
        }
    }

    #[test]
    fn test_pending_by_sender() {
        let mut monitor = MempoolMonitor::new();
        monitor
            .add_tx(make_tx("h1", "alice", "bob", 500, "transfer"))
            .unwrap();
        monitor
            .add_tx(make_tx("h2", "alice", "carol", 600, "transfer"))
            .unwrap();
        monitor
            .add_tx(make_tx("h3", "bob", "carol", 700, "transfer"))
            .unwrap();

        let alice_txs = monitor.pending_by_sender("alice");
        assert_eq!(alice_txs.len(), 2);
        assert!(alice_txs.iter().all(|tx| tx.sender == "alice"));
    }

    #[test]
    fn test_pending_by_priority() {
        let mut monitor = MempoolMonitor::new();
        monitor.add_tx(make_tx("h1", "a", "b", 500, "t")).unwrap(); // Low
        monitor.add_tx(make_tx("h2", "a", "b", 1000, "t")).unwrap(); // Medium
        monitor.add_tx(make_tx("h3", "a", "b", 5000, "t")).unwrap(); // High
        monitor.add_tx(make_tx("h4", "a", "b", 10000, "t")).unwrap(); // Urgent

        let low = monitor.pending_by_priority(&TxPriority::Low);
        assert_eq!(low.len(), 1);
        assert_eq!(low[0].tx_hash, "h1");

        let urgent = monitor.pending_by_priority(&TxPriority::Urgent);
        assert_eq!(urgent.len(), 1);
        assert_eq!(urgent[0].tx_hash, "h4");
    }

    #[test]
    fn test_highest_and_lowest_fee() {
        let mut monitor = MempoolMonitor::new();
        monitor.add_tx(make_tx("h1", "a", "b", 100, "t")).unwrap();
        monitor.add_tx(make_tx("h2", "a", "b", 9999, "t")).unwrap();
        monitor.add_tx(make_tx("h3", "a", "b", 500, "t")).unwrap();

        assert_eq!(monitor.highest_fee().unwrap().fee, 9999);
        assert_eq!(monitor.lowest_fee().unwrap().fee, 100);
    }

    #[test]
    fn test_congestion_levels() {
        let mut monitor = MempoolMonitor::new();
        assert_eq!(monitor.congestion(), CongestionLevel::Empty);

        // Add 5 => Low
        for i in 0..5 {
            monitor
                .add_tx(make_tx(&format!("h{}", i), "a", "b", 100, "t"))
                .unwrap();
        }
        assert_eq!(monitor.congestion(), CongestionLevel::Low);

        // Add to 15 => Normal
        for i in 5..15 {
            monitor
                .add_tx(make_tx(&format!("h{}", i), "a", "b", 100, "t"))
                .unwrap();
        }
        assert_eq!(monitor.congestion(), CongestionLevel::Normal);

        // Add to 100 => High
        for i in 15..100 {
            monitor
                .add_tx(make_tx(&format!("h{}", i), "a", "b", 100, "t"))
                .unwrap();
        }
        assert_eq!(monitor.congestion(), CongestionLevel::High);

        // Add to 200 => Critical
        for i in 100..200 {
            monitor
                .add_tx(make_tx(&format!("h{}", i), "a", "b", 100, "t"))
                .unwrap();
        }
        assert_eq!(monitor.congestion(), CongestionLevel::Critical);
    }

    #[test]
    fn test_detect_front_run_high() {
        let mut monitor = MempoolMonitor::new();
        monitor
            .add_tx(make_tx("victim", "alice", "contract1", 1000, "swap"))
            .unwrap();
        monitor
            .add_tx(make_tx("attacker", "eve", "contract1", 5000, "swap"))
            .unwrap();

        let risk = monitor.detect_front_run("victim", "attacker").unwrap();
        assert_eq!(risk, FrontRunRisk::High);
        assert_eq!(monitor.alerts.len(), 1);
        assert_eq!(monitor.alerts[0].risk, FrontRunRisk::High);
    }

    #[test]
    fn test_detect_front_run_medium() {
        let mut monitor = MempoolMonitor::new();
        monitor
            .add_tx(make_tx("victim", "alice", "contract1", 1000, "swap"))
            .unwrap();
        monitor
            .add_tx(make_tx("attacker", "eve", "contract2", 5000, "swap"))
            .unwrap();

        let risk = monitor.detect_front_run("victim", "attacker").unwrap();
        assert_eq!(risk, FrontRunRisk::Medium);
    }

    #[test]
    fn test_detect_front_run_low() {
        let mut monitor = MempoolMonitor::new();
        monitor
            .add_tx(make_tx("victim", "alice", "contract1", 1000, "swap"))
            .unwrap();
        monitor
            .add_tx(make_tx("attacker", "eve", "contract2", 5000, "transfer"))
            .unwrap();

        let risk = monitor.detect_front_run("victim", "attacker").unwrap();
        assert_eq!(risk, FrontRunRisk::Low);
    }

    #[test]
    fn test_detect_front_run_not_found() {
        let mut monitor = MempoolMonitor::new();
        monitor
            .add_tx(make_tx("victim", "alice", "bob", 1000, "t"))
            .unwrap();

        let result = monitor.detect_front_run("victim", "nonexistent");
        assert!(result.is_err());

        let result = monitor.detect_front_run("nonexistent", "victim");
        assert!(result.is_err());
    }

    #[test]
    fn test_gas_oracle_recommend() {
        let mut oracle = GasOracle::new();
        for fee in [100, 200, 300, 400, 500, 600, 700, 800, 900, 1000] {
            oracle.record(fee, true);
        }

        let low = oracle.recommend(&TxPriority::Low);
        let medium = oracle.recommend(&TxPriority::Medium);
        let high = oracle.recommend(&TxPriority::High);
        let urgent = oracle.recommend(&TxPriority::Urgent);

        assert!(low <= medium);
        assert!(medium <= high);
        assert!(high <= urgent);
    }

    #[test]
    fn test_gas_oracle_percentile() {
        let mut oracle = GasOracle::new();
        for fee in 1..=100 {
            oracle.record(fee, true);
        }

        let p50 = oracle.percentile(50.0);
        assert!((49..=51).contains(&p50));

        let p0 = oracle.percentile(0.0);
        assert_eq!(p0, 1);

        let p100 = oracle.percentile(100.0);
        assert_eq!(p100, 100);
    }

    #[test]
    fn test_gas_oracle_empty() {
        let oracle = GasOracle::new();
        assert_eq!(oracle.recommend(&TxPriority::Medium), 100);
        assert_eq!(oracle.percentile(50.0), 0);
        assert_eq!(oracle.average_fee(), 0);
        assert_eq!(oracle.sample_count(), 0);
    }

    #[test]
    fn test_record_inclusion() {
        let mut monitor = MempoolMonitor::new();
        monitor.record_inclusion(500);
        monitor.record_inclusion(1000);

        assert_eq!(monitor.oracle.sample_count(), 2);
        assert!(monitor.oracle.samples.iter().all(|s| s.included));
    }

    #[test]
    fn test_stale_txs() {
        let mut monitor = MempoolMonitor::new();
        let mut tx = make_tx("old_tx", "alice", "bob", 500, "transfer");
        tx.first_seen = "2020-01-01T00:00:00+00:00".to_string();
        monitor.add_tx(tx).unwrap();

        let fresh_tx = make_tx("new_tx", "alice", "bob", 500, "transfer");
        monitor.add_tx(fresh_tx).unwrap();

        let stale = monitor.stale_txs(3600);
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].tx_hash, "old_tx");
    }

    #[test]
    fn test_fee_histogram() {
        let mut monitor = MempoolMonitor::new();
        monitor.add_tx(make_tx("h1", "a", "b", 100, "t")).unwrap();
        monitor.add_tx(make_tx("h2", "a", "b", 500, "t")).unwrap();
        monitor.add_tx(make_tx("h3", "a", "b", 1000, "t")).unwrap();
        monitor.add_tx(make_tx("h4", "a", "b", 5000, "t")).unwrap();

        let hist = monitor.fee_histogram(&[200, 600, 2000, 10000]);
        assert_eq!(hist, vec![(200, 1), (600, 2), (2000, 3), (10000, 4)]);
    }

    #[test]
    fn test_clear_pending() {
        let mut monitor = MempoolMonitor::new();
        monitor.add_tx(make_tx("h1", "a", "b", 100, "t")).unwrap();
        monitor.add_tx(make_tx("h2", "a", "b", 200, "t")).unwrap();
        monitor.add_tx(make_tx("h3", "a", "b", 300, "t")).unwrap();

        let cleared = monitor.clear_pending();
        assert_eq!(cleared, 3);
        assert_eq!(monitor.pending_count(), 0);
    }

    #[test]
    fn test_stats() {
        let mut monitor = MempoolMonitor::new();
        monitor.add_tx(make_tx("h1", "a", "b", 100, "t")).unwrap();
        monitor.add_tx(make_tx("h2", "a", "b", 300, "t")).unwrap();
        monitor.remove_tx("h1").unwrap();

        let stats = monitor.stats();
        assert_eq!(stats.pending_count, 1);
        assert_eq!(stats.total_fees, 300);
        assert_eq!(stats.avg_fee, 300);
        assert_eq!(stats.removed_count, 1);
        assert_eq!(stats.alert_count, 0);
        assert_eq!(stats.congestion, CongestionLevel::Low);
    }

    #[test]
    fn test_persistence_roundtrip() {
        let path = test_path("roundtrip");

        let mut monitor = MempoolMonitor::new();
        monitor
            .add_tx(make_tx("h1", "alice", "bob", 5000, "transfer"))
            .unwrap();
        monitor
            .add_tx(make_tx("h2", "carol", "dave", 1000, "swap"))
            .unwrap();
        monitor.record_inclusion(800);
        monitor.remove_tx("h1").unwrap();

        monitor.save(&path).unwrap();

        let loaded = MempoolMonitor::load(&path).unwrap();
        assert_eq!(loaded.pending_count(), 1);
        assert_eq!(loaded.removed_count, 1);
        assert_eq!(loaded.oracle.sample_count(), 1);
        assert!(loaded.get_tx("h2").is_some());
        assert!(loaded.get_tx("h1").is_none());

        // Clean up
        let _ = std::fs::remove_file(&path);

        // Test load_or_default on missing file
        let default = MempoolMonitor::load_or_default(Path::new("/tmp/nonexistent_mempool_file"));
        assert_eq!(default.pending_count(), 0);
    }

    #[test]
    fn test_age_secs_invalid_timestamp_covers_lines_116_117() {
        let tx = make_tx("h1", "a", "b", 500, "transfer");
        // Set invalid timestamp so fallback to epoch fires
        let mut tx = tx;
        tx.first_seen = "not-a-date".to_string();
        let age = tx.age_secs();
        // Should be a very large number (years since 1970)
        assert!(age > 0);
    }

    #[test]
    fn test_fee_per_byte_covers_lines_124_126() {
        let tx = make_tx("h1", "a", "b", 100, "transfer"); // fee=100, 100/250=0, max(1)=1
        assert_eq!(tx.fee_per_byte(), 1);
        let tx2 = make_tx("h2", "a", "b", 500, "transfer"); // fee=500, 500/250=2
        assert_eq!(tx2.fee_per_byte(), 2);
    }

    #[test]
    fn test_average_fee_with_samples_covers_lines_197_199() {
        let mut oracle = GasOracle::new();
        oracle.record(100, true);
        oracle.record(200, true);
        oracle.record(300, true);
        assert_eq!(oracle.average_fee(), 200);
    }

    #[test]
    fn test_gas_oracle_default_covers_lines_209_211() {
        let oracle = GasOracle::default();
        assert_eq!(oracle.sample_count(), 0);
    }

    #[test]
    fn test_alerts_overflow_covers_line_366() {
        let mut monitor = MempoolMonitor::new();
        monitor.max_alerts = 2;
        monitor.add_tx(make_tx("v", "a", "contract", 1000, "swap")).unwrap();
        // Add 3 attackers to overflow max_alerts
        for i in 0..3 {
            let hash = format!("atk{}", i);
            monitor.add_tx(make_tx(&hash, "e", "contract", 5000, "swap")).unwrap();
            let _ = monitor.detect_front_run("v", &hash);
        }
        assert!(monitor.alerts.len() <= monitor.max_alerts);
    }

    #[test]
    fn test_recommend_fee_covers_lines_389_391() {
        let mut monitor = MempoolMonitor::new();
        monitor.record_inclusion(1000);
        monitor.record_inclusion(2000);
        let fee = monitor.recommend_fee(&TxPriority::Urgent);
        assert!(fee > 0);
        let fee_low = monitor.recommend_fee(&TxPriority::Low);
        assert!(fee_low <= fee);
    }

    #[test]
    fn test_stats_empty_pending_covers_line_402() {
        let monitor = MempoolMonitor::new();
        let stats = monitor.stats();
        assert_eq!(stats.pending_count, 0);
        assert_eq!(stats.avg_fee, 0);
    }
}
