//! Stuck transaction detector and fee bumper.
//!
//! Monitors pending transactions and automatically bumps fees when
//! they appear stuck. Supports multiple bump strategies:
//! - Linear: fixed increment each bump
//! - Percentage: increase by a percentage each bump
//! - Aggressive: double the fee each bump
//! - Custom: set an explicit new fee
//!
//! All state is persisted as JSON for crash recovery.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum FeeBumperError {
    #[error("transaction already tracked: {0}")]
    AlreadyTracked(String),
    #[error("transaction not found: {0}")]
    NotFound(String),
    #[error("max bumps reached ({0})")]
    MaxBumpsReached(u32),
    #[error("invalid state: {0}")]
    InvalidState(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON parse error: {0}")]
    Parse(#[from] serde_json::Error),
}

// ──────────────────────────── Enums ──────────────────────────────────────

/// State of a tracked transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TxState {
    Pending,
    Confirmed,
    Stuck,
    Bumped,
    Failed,
    Dropped,
}

/// Strategy for bumping a stuck transaction's fee.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "type", content = "value")]
pub enum BumpStrategy {
    /// Add a fixed increment each bump.
    Linear(u64),
    /// Increase by a percentage (e.g. 25.0 = +25%).
    Percentage(f64),
    /// Double the fee each bump.
    Aggressive,
    /// Set an explicit new fee.
    Custom(u64),
}

/// Why a transaction was detected as stuck.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StuckReason {
    LowFee,
    NetworkCongestion,
    NonceTooHigh,
    Unknown,
}

// ──────────────────────────── BumpRecord ──────────────────────────────────

/// Record of a single fee bump.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BumpRecord {
    pub from_fee: u64,
    pub to_fee: u64,
    pub strategy: String,
    pub bumped_at: String,
}

// ──────────────────────────── TrackedTx ──────────────────────────────────

/// A transaction being monitored for stuck detection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackedTx {
    pub tx_hash: String,
    pub sender: String,
    pub nonce: u64,
    pub original_fee: u64,
    pub current_fee: u64,
    pub state: TxState,
    pub submitted_at: String,
    pub last_checked: String,
    pub bump_count: u32,
    pub max_bumps: u32,
    pub bump_history: Vec<BumpRecord>,
    pub stuck_reason: Option<StuckReason>,
    pub replacement_hash: Option<String>,
}

impl TrackedTx {
    /// Create a new tracked transaction in Pending state.
    pub fn new(
        tx_hash: impl Into<String>,
        sender: impl Into<String>,
        nonce: u64,
        fee: u64,
    ) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            tx_hash: tx_hash.into(),
            sender: sender.into(),
            nonce,
            original_fee: fee,
            current_fee: fee,
            state: TxState::Pending,
            submitted_at: now.clone(),
            last_checked: now,
            bump_count: 0,
            max_bumps: 5,
            bump_history: Vec::new(),
            stuck_reason: None,
            replacement_hash: None,
        }
    }

    /// True if the transaction is Pending and has exceeded the timeout.
    pub fn is_stuck(&self, timeout_secs: u64) -> bool {
        matches!(self.state, TxState::Pending) && self.time_pending_secs() > timeout_secs
    }

    /// Seconds elapsed since the transaction was submitted.
    pub fn time_pending_secs(&self) -> u64 {
        let submitted =
            chrono::DateTime::parse_from_rfc3339(&self.submitted_at).unwrap_or_else(|_| {
                chrono::DateTime::parse_from_rfc3339("1970-01-01T00:00:00+00:00").unwrap()
            });
        let now = chrono::Utc::now();
        let diff = now.signed_duration_since(submitted);
        diff.num_seconds().max(0) as u64
    }

    /// Bump the fee using the given strategy.
    ///
    /// Returns the new fee on success. Fails if max bumps reached or
    /// the transaction is not in a bumpable state (Pending or Stuck).
    pub fn bump_fee(&mut self, strategy: &BumpStrategy) -> Result<u64, FeeBumperError> {
        if self.bump_count >= self.max_bumps {
            return Err(FeeBumperError::MaxBumpsReached(self.max_bumps));
        }
        if !matches!(self.state, TxState::Pending | TxState::Stuck) {
            return Err(FeeBumperError::InvalidState(format!(
                "cannot bump tx in state {:?}",
                self.state
            )));
        }

        let old_fee = self.current_fee;
        let new_fee = match strategy {
            BumpStrategy::Linear(increment) => old_fee + increment,
            BumpStrategy::Percentage(pct) => {
                let increase = (old_fee as f64 * pct / 100.0).ceil() as u64;
                old_fee + increase
            }
            BumpStrategy::Aggressive => old_fee * 2,
            BumpStrategy::Custom(fee) => *fee,
        };

        let strategy_name = match strategy {
            BumpStrategy::Linear(v) => format!("linear({})", v),
            BumpStrategy::Percentage(v) => format!("percentage({}%)", v),
            BumpStrategy::Aggressive => "aggressive".to_string(),
            BumpStrategy::Custom(v) => format!("custom({})", v),
        };

        self.bump_history.push(BumpRecord {
            from_fee: old_fee,
            to_fee: new_fee,
            strategy: strategy_name,
            bumped_at: chrono::Utc::now().to_rfc3339(),
        });

        self.current_fee = new_fee;
        self.bump_count += 1;
        self.state = TxState::Bumped;
        self.last_checked = chrono::Utc::now().to_rfc3339();

        Ok(new_fee)
    }

    /// Mark the transaction as confirmed.
    pub fn mark_confirmed(&mut self) {
        self.state = TxState::Confirmed;
        self.last_checked = chrono::Utc::now().to_rfc3339();
    }

    /// Mark the transaction as failed with a reason.
    pub fn mark_failed(&mut self, reason: &str) {
        self.state = TxState::Failed;
        self.stuck_reason = Some(StuckReason::Unknown);
        self.replacement_hash = Some(reason.to_string());
        self.last_checked = chrono::Utc::now().to_rfc3339();
    }

    /// Mark the transaction as dropped.
    pub fn mark_dropped(&mut self) {
        self.state = TxState::Dropped;
        self.last_checked = chrono::Utc::now().to_rfc3339();
    }

    /// Current effective fee.
    pub fn effective_fee(&self) -> u64 {
        self.current_fee
    }

    /// Total fee increase from the original.
    pub fn total_fee_increase(&self) -> u64 {
        self.current_fee.saturating_sub(self.original_fee)
    }
}

// ──────────────────────────── BumperStats ────────────────────────────────

/// Summary statistics for the fee bumper.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BumperStats {
    pub total_tracked: u64,
    pub pending: u64,
    pub stuck: u64,
    pub confirmed: u64,
    pub failed: u64,
    pub total_bumps: u64,
    pub total_fee_spent: u64,
}

// ──────────────────────────── FeeBumper ──────────────────────────────────

/// Detects stuck transactions and bumps their fees.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeeBumper {
    pub tracked: HashMap<String, TrackedTx>,
    pub default_strategy: BumpStrategy,
    pub default_timeout_secs: u64,
    pub auto_bump_enabled: bool,
    pub total_bumps: u64,
    pub total_fee_spent: u64,
}

impl Default for FeeBumper {
    fn default() -> Self {
        Self::new()
    }
}

impl FeeBumper {
    /// Create a new fee bumper with default settings.
    pub fn new() -> Self {
        Self {
            tracked: HashMap::new(),
            default_strategy: BumpStrategy::Percentage(25.0),
            default_timeout_secs: 120,
            auto_bump_enabled: false,
            total_bumps: 0,
            total_fee_spent: 0,
        }
    }

    /// Set the default bump strategy (builder pattern).
    pub fn with_strategy(mut self, strategy: BumpStrategy) -> Self {
        self.default_strategy = strategy;
        self
    }

    /// Set the default stuck timeout in seconds (builder pattern).
    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.default_timeout_secs = secs;
        self
    }

    /// Start tracking a transaction. Fails if already tracked.
    pub fn track(&mut self, tx: TrackedTx) -> Result<(), FeeBumperError> {
        if self.tracked.contains_key(&tx.tx_hash) {
            return Err(FeeBumperError::AlreadyTracked(tx.tx_hash.clone()));
        }
        self.total_fee_spent += tx.current_fee;
        self.tracked.insert(tx.tx_hash.clone(), tx);
        Ok(())
    }

    /// Stop tracking a transaction. Returns the removed tx.
    pub fn untrack(&mut self, tx_hash: &str) -> Result<TrackedTx, FeeBumperError> {
        self.tracked
            .remove(tx_hash)
            .ok_or_else(|| FeeBumperError::NotFound(tx_hash.to_string()))
    }

    /// Get a reference to a tracked transaction.
    pub fn get(&self, tx_hash: &str) -> Option<&TrackedTx> {
        self.tracked.get(tx_hash)
    }

    /// Get a mutable reference to a tracked transaction.
    pub fn get_mut(&mut self, tx_hash: &str) -> Option<&mut TrackedTx> {
        self.tracked.get_mut(tx_hash)
    }

    /// Detect stuck transactions (pending longer than timeout).
    ///
    /// Updates their state to Stuck with reason LowFee. Returns hashes
    /// of newly-stuck transactions.
    pub fn detect_stuck(&mut self) -> Vec<String> {
        let timeout = self.default_timeout_secs;
        let stuck: Vec<String> = self
            .tracked
            .iter()
            .filter(|(_, tx)| tx.is_stuck(timeout))
            .map(|(hash, _)| hash.clone())
            .collect();

        for hash in &stuck {
            if let Some(tx) = self.tracked.get_mut(hash) {
                tx.state = TxState::Stuck;
                tx.stuck_reason = Some(StuckReason::LowFee);
                tx.last_checked = chrono::Utc::now().to_rfc3339();
            }
        }

        stuck
    }

    /// Bump the fee for a specific transaction.
    ///
    /// Uses the provided strategy or falls back to the default. Returns the new fee.
    pub fn bump(
        &mut self,
        tx_hash: &str,
        strategy: Option<BumpStrategy>,
    ) -> Result<u64, FeeBumperError> {
        let strat = strategy.unwrap_or_else(|| self.default_strategy.clone());
        let tx = self
            .tracked
            .get_mut(tx_hash)
            .ok_or_else(|| FeeBumperError::NotFound(tx_hash.to_string()))?;

        let old_fee = tx.current_fee;
        let new_fee = tx.bump_fee(&strat)?;
        let fee_diff = new_fee.saturating_sub(old_fee);
        self.total_bumps += 1;
        self.total_fee_spent += fee_diff;
        Ok(new_fee)
    }

    /// Bump all stuck transactions using the default strategy.
    ///
    /// Returns a vec of (tx_hash, new_fee) for each bumped tx.
    pub fn bump_all_stuck(&mut self) -> Vec<(String, u64)> {
        let stuck_hashes: Vec<String> = self
            .tracked
            .iter()
            .filter(|(_, tx)| matches!(tx.state, TxState::Stuck))
            .map(|(hash, _)| hash.clone())
            .collect();

        let mut results = Vec::new();
        for hash in stuck_hashes {
            if let Ok(new_fee) = self.bump(&hash, None) {
                results.push((hash, new_fee));
            }
        }
        results
    }

    /// All pending transactions.
    pub fn pending_txs(&self) -> Vec<&TrackedTx> {
        self.tracked
            .values()
            .filter(|tx| matches!(tx.state, TxState::Pending))
            .collect()
    }

    /// All stuck transactions.
    pub fn stuck_txs(&self) -> Vec<&TrackedTx> {
        self.tracked
            .values()
            .filter(|tx| matches!(tx.state, TxState::Stuck))
            .collect()
    }

    /// All confirmed transactions.
    pub fn confirmed_txs(&self) -> Vec<&TrackedTx> {
        self.tracked
            .values()
            .filter(|tx| matches!(tx.state, TxState::Confirmed))
            .collect()
    }

    /// Compute summary statistics.
    pub fn stats(&self) -> BumperStats {
        let mut pending = 0u64;
        let mut stuck = 0u64;
        let mut confirmed = 0u64;
        let mut failed = 0u64;
        for tx in self.tracked.values() {
            match tx.state {
                TxState::Pending => pending += 1,
                TxState::Stuck => stuck += 1,
                TxState::Confirmed => confirmed += 1,
                TxState::Failed => failed += 1,
                _ => {}
            }
        }
        BumperStats {
            total_tracked: self.tracked.len() as u64,
            pending,
            stuck,
            confirmed,
            failed,
            total_bumps: self.total_bumps,
            total_fee_spent: self.total_fee_spent,
        }
    }

    /// Remove confirmed transactions. Returns count removed.
    pub fn cleanup_confirmed(&mut self) -> usize {
        let before = self.tracked.len();
        self.tracked
            .retain(|_, tx| !matches!(tx.state, TxState::Confirmed));
        before - self.tracked.len()
    }

    /// Get all tracked transactions from a specific sender.
    pub fn by_sender(&self, sender: &str) -> Vec<&TrackedTx> {
        self.tracked
            .values()
            .filter(|tx| tx.sender == sender)
            .collect()
    }

    /// Find nonce gaps in a sender's pending transactions.
    ///
    /// Returns the missing nonce values between the lowest and highest
    /// nonces for that sender's pending/stuck transactions.
    pub fn nonce_gaps(&self, sender: &str) -> Vec<u64> {
        let mut nonces: Vec<u64> = self
            .tracked
            .values()
            .filter(|tx| {
                tx.sender == sender
                    && matches!(
                        tx.state,
                        TxState::Pending | TxState::Stuck | TxState::Bumped
                    )
            })
            .map(|tx| tx.nonce)
            .collect();

        if nonces.len() < 2 {
            return Vec::new();
        }

        nonces.sort_unstable();
        nonces.dedup();

        let min = nonces[0];
        let max = *nonces.last().unwrap();
        let nonce_set: std::collections::HashSet<u64> = nonces.into_iter().collect();

        (min..=max).filter(|n| !nonce_set.contains(n)).collect()
    }

    /// Save state to a JSON file.
    pub fn save(&self, path: &Path) -> Result<(), FeeBumperError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Load state from a JSON file.
    pub fn load(path: &Path) -> Result<Self, FeeBumperError> {
        let data = std::fs::read_to_string(path)?;
        let bumper: FeeBumper = serde_json::from_str(&data)?;
        Ok(bumper)
    }

    /// Load from file, or return default if file doesn't exist or is invalid.
    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ──────────────────────────── Tests ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tx(hash: &str, sender: &str, nonce: u64, fee: u64) -> TrackedTx {
        TrackedTx::new(hash, sender, nonce, fee)
    }

    fn make_old_tx(hash: &str, sender: &str, nonce: u64, fee: u64) -> TrackedTx {
        let mut tx = TrackedTx::new(hash, sender, nonce, fee);
        tx.submitted_at = "2020-01-01T00:00:00+00:00".to_string();
        tx
    }

    fn test_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("fee_bumper_test_{}_{}", name, std::process::id()))
    }

    // ── track / get ──

    #[test]
    fn test_track_and_get() {
        let mut bumper = FeeBumper::new();
        let tx = make_tx("0xaaa", "alice", 0, 1000);
        bumper.track(tx).unwrap();

        let got = bumper.get("0xaaa").unwrap();
        assert_eq!(got.tx_hash, "0xaaa");
        assert_eq!(got.sender, "alice");
        assert_eq!(got.nonce, 0);
        assert_eq!(got.original_fee, 1000);
        assert_eq!(got.state, TxState::Pending);
    }

    #[test]
    fn test_track_duplicate_rejected() {
        let mut bumper = FeeBumper::new();
        bumper.track(make_tx("0xaaa", "alice", 0, 1000)).unwrap();
        let result = bumper.track(make_tx("0xaaa", "alice", 0, 2000));
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            FeeBumperError::AlreadyTracked(_)
        ));
    }

    #[test]
    fn test_untrack() {
        let mut bumper = FeeBumper::new();
        bumper.track(make_tx("0xaaa", "alice", 0, 1000)).unwrap();
        let removed = bumper.untrack("0xaaa").unwrap();
        assert_eq!(removed.tx_hash, "0xaaa");
        assert!(bumper.get("0xaaa").is_none());
    }

    #[test]
    fn test_untrack_not_found() {
        let mut bumper = FeeBumper::new();
        let result = bumper.untrack("0xnope");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), FeeBumperError::NotFound(_)));
    }

    // ── stuck detection ──

    #[test]
    fn test_is_stuck() {
        let tx = make_old_tx("0xaaa", "alice", 0, 1000);
        assert!(tx.is_stuck(120));
    }

    #[test]
    fn test_not_stuck_yet() {
        let tx = make_tx("0xaaa", "alice", 0, 1000);
        assert!(!tx.is_stuck(120));
    }

    // ── bump strategies ──

    #[test]
    fn test_bump_fee_linear() {
        let mut tx = make_old_tx("0xaaa", "alice", 0, 1000);
        tx.state = TxState::Stuck;
        let new_fee = tx.bump_fee(&BumpStrategy::Linear(200)).unwrap();
        assert_eq!(new_fee, 1200);
        assert_eq!(tx.current_fee, 1200);
        assert_eq!(tx.bump_count, 1);
    }

    #[test]
    fn test_bump_fee_percentage() {
        let mut tx = make_old_tx("0xaaa", "alice", 0, 1000);
        tx.state = TxState::Stuck;
        let new_fee = tx.bump_fee(&BumpStrategy::Percentage(25.0)).unwrap();
        assert_eq!(new_fee, 1250);
        assert_eq!(tx.current_fee, 1250);
    }

    #[test]
    fn test_bump_fee_aggressive() {
        let mut tx = make_old_tx("0xaaa", "alice", 0, 1000);
        tx.state = TxState::Stuck;

        let fee1 = tx.bump_fee(&BumpStrategy::Aggressive).unwrap();
        assert_eq!(fee1, 2000);

        // After first bump state is Bumped, set back to Stuck for second bump
        tx.state = TxState::Stuck;
        let fee2 = tx.bump_fee(&BumpStrategy::Aggressive).unwrap();
        assert_eq!(fee2, 4000);
    }

    #[test]
    fn test_bump_fee_custom() {
        let mut tx = make_old_tx("0xaaa", "alice", 0, 1000);
        tx.state = TxState::Stuck;
        let new_fee = tx.bump_fee(&BumpStrategy::Custom(5000)).unwrap();
        assert_eq!(new_fee, 5000);
        assert_eq!(tx.current_fee, 5000);
    }

    #[test]
    fn test_max_bumps_reached() {
        let mut tx = make_old_tx("0xaaa", "alice", 0, 1000);
        tx.max_bumps = 2;
        tx.state = TxState::Stuck;

        tx.bump_fee(&BumpStrategy::Linear(100)).unwrap();
        tx.state = TxState::Stuck;
        tx.bump_fee(&BumpStrategy::Linear(100)).unwrap();
        tx.state = TxState::Stuck;

        let result = tx.bump_fee(&BumpStrategy::Linear(100));
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            FeeBumperError::MaxBumpsReached(2)
        ));
    }

    #[test]
    fn test_bump_invalid_state() {
        let mut tx = make_tx("0xaaa", "alice", 0, 1000);
        tx.mark_confirmed();
        let result = tx.bump_fee(&BumpStrategy::Linear(100));
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            FeeBumperError::InvalidState(_)
        ));
    }

    // ── FeeBumper methods ──

    #[test]
    fn test_detect_stuck() {
        let mut bumper = FeeBumper::new().with_timeout(60);

        bumper
            .track(make_old_tx("0xold", "alice", 0, 1000))
            .unwrap();
        bumper.track(make_tx("0xnew", "alice", 1, 2000)).unwrap();

        let stuck = bumper.detect_stuck();
        assert_eq!(stuck.len(), 1);
        assert_eq!(stuck[0], "0xold");
        assert_eq!(bumper.get("0xold").unwrap().state, TxState::Stuck);
        assert_eq!(
            bumper.get("0xold").unwrap().stuck_reason,
            Some(StuckReason::LowFee)
        );
    }

    #[test]
    fn test_bump_all_stuck() {
        let mut bumper = FeeBumper::new()
            .with_strategy(BumpStrategy::Linear(500))
            .with_timeout(60);

        let mut tx1 = make_old_tx("0xa", "alice", 0, 1000);
        tx1.state = TxState::Stuck;
        tx1.stuck_reason = Some(StuckReason::LowFee);
        bumper.track(tx1).unwrap();

        let mut tx2 = make_old_tx("0xb", "bob", 0, 2000);
        tx2.state = TxState::Stuck;
        tx2.stuck_reason = Some(StuckReason::LowFee);
        bumper.track(tx2).unwrap();

        let results = bumper.bump_all_stuck();
        assert_eq!(results.len(), 2);

        for (hash, new_fee) in &results {
            match hash.as_str() {
                "0xa" => assert_eq!(*new_fee, 1500),
                "0xb" => assert_eq!(*new_fee, 2500),
                _ => panic!("unexpected hash"),
            }
        }
    }

    #[test]
    fn test_mark_confirmed() {
        let mut tx = make_tx("0xaaa", "alice", 0, 1000);
        tx.mark_confirmed();
        assert_eq!(tx.state, TxState::Confirmed);
    }

    #[test]
    fn test_mark_failed() {
        let mut tx = make_tx("0xaaa", "alice", 0, 1000);
        tx.mark_failed("out of gas");
        assert_eq!(tx.state, TxState::Failed);
    }

    #[test]
    fn test_mark_dropped() {
        let mut tx = make_tx("0xaaa", "alice", 0, 1000);
        tx.mark_dropped();
        assert_eq!(tx.state, TxState::Dropped);
    }

    #[test]
    fn test_pending_txs() {
        let mut bumper = FeeBumper::new();
        bumper.track(make_tx("0xa", "alice", 0, 100)).unwrap();
        bumper.track(make_tx("0xb", "bob", 0, 200)).unwrap();

        let mut tx_c = make_tx("0xc", "carol", 0, 300);
        tx_c.mark_confirmed();
        bumper.track(tx_c).unwrap();

        let pending = bumper.pending_txs();
        assert_eq!(pending.len(), 2);
    }

    #[test]
    fn test_stuck_txs() {
        let mut bumper = FeeBumper::new().with_timeout(60);

        bumper.track(make_old_tx("0xa", "alice", 0, 1000)).unwrap();
        bumper.track(make_tx("0xb", "bob", 0, 2000)).unwrap();

        bumper.detect_stuck();

        let stuck = bumper.stuck_txs();
        assert_eq!(stuck.len(), 1);
        assert_eq!(stuck[0].tx_hash, "0xa");
    }

    #[test]
    fn test_cleanup_confirmed() {
        let mut bumper = FeeBumper::new();
        bumper.track(make_tx("0xa", "alice", 0, 100)).unwrap();

        let mut tx_b = make_tx("0xb", "bob", 0, 200);
        tx_b.mark_confirmed();
        bumper.track(tx_b).unwrap();

        let mut tx_c = make_tx("0xc", "carol", 0, 300);
        tx_c.mark_confirmed();
        bumper.track(tx_c).unwrap();

        let removed = bumper.cleanup_confirmed();
        assert_eq!(removed, 2);
        assert_eq!(bumper.tracked.len(), 1);
        assert!(bumper.get("0xa").is_some());
    }

    #[test]
    fn test_by_sender() {
        let mut bumper = FeeBumper::new();
        bumper.track(make_tx("0xa", "alice", 0, 100)).unwrap();
        bumper.track(make_tx("0xb", "alice", 1, 200)).unwrap();
        bumper.track(make_tx("0xc", "bob", 0, 300)).unwrap();

        let alice_txs = bumper.by_sender("alice");
        assert_eq!(alice_txs.len(), 2);
    }

    #[test]
    fn test_nonce_gaps() {
        let mut bumper = FeeBumper::new();
        bumper.track(make_tx("0xa", "alice", 0, 100)).unwrap();
        bumper.track(make_tx("0xb", "alice", 1, 200)).unwrap();
        // gap at nonce 2
        bumper.track(make_tx("0xc", "alice", 3, 300)).unwrap();
        bumper.track(make_tx("0xd", "alice", 5, 400)).unwrap();
        // gap at nonce 4

        let gaps = bumper.nonce_gaps("alice");
        assert_eq!(gaps, vec![2, 4]);
    }

    #[test]
    fn test_stats() {
        let mut bumper = FeeBumper::new();
        bumper.track(make_tx("0xa", "alice", 0, 100)).unwrap();
        bumper.track(make_tx("0xb", "bob", 0, 200)).unwrap();

        let mut tx_c = make_tx("0xc", "carol", 0, 300);
        tx_c.mark_confirmed();
        bumper.track(tx_c).unwrap();

        let mut tx_d = make_tx("0xd", "dave", 0, 400);
        tx_d.mark_failed("reason");
        bumper.track(tx_d).unwrap();

        let stats = bumper.stats();
        assert_eq!(stats.total_tracked, 4);
        assert_eq!(stats.pending, 2);
        assert_eq!(stats.confirmed, 1);
        assert_eq!(stats.failed, 1);
    }

    #[test]
    fn test_persistence_roundtrip() {
        let path = test_path("roundtrip");
        let mut bumper = FeeBumper::new()
            .with_strategy(BumpStrategy::Linear(100))
            .with_timeout(300);
        bumper.track(make_tx("0xaaa", "alice", 0, 1000)).unwrap();
        bumper.track(make_tx("0xbbb", "bob", 0, 2000)).unwrap();

        bumper.save(&path).unwrap();
        let loaded = FeeBumper::load(&path).unwrap();

        assert_eq!(loaded.tracked.len(), 2);
        assert_eq!(loaded.default_timeout_secs, 300);
        assert!(loaded.get("0xaaa").is_some());
        assert!(loaded.get("0xbbb").is_some());
        assert_eq!(loaded.get("0xaaa").unwrap().original_fee, 1000);

        // cleanup
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_total_fee_increase() {
        let mut tx = make_old_tx("0xaaa", "alice", 0, 1000);
        assert_eq!(tx.total_fee_increase(), 0);

        tx.state = TxState::Stuck;
        tx.bump_fee(&BumpStrategy::Linear(300)).unwrap();
        assert_eq!(tx.total_fee_increase(), 300);
        assert_eq!(tx.effective_fee(), 1300);
    }

    #[test]
    fn test_bump_history() {
        let mut tx = make_old_tx("0xaaa", "alice", 0, 1000);
        tx.state = TxState::Stuck;

        tx.bump_fee(&BumpStrategy::Linear(100)).unwrap();
        tx.state = TxState::Stuck;
        tx.bump_fee(&BumpStrategy::Percentage(50.0)).unwrap();

        assert_eq!(tx.bump_history.len(), 2);

        assert_eq!(tx.bump_history[0].from_fee, 1000);
        assert_eq!(tx.bump_history[0].to_fee, 1100);
        assert!(tx.bump_history[0].strategy.contains("linear"));

        assert_eq!(tx.bump_history[1].from_fee, 1100);
        assert_eq!(tx.bump_history[1].to_fee, 1650);
        assert!(tx.bump_history[1].strategy.contains("percentage"));
    }
}
