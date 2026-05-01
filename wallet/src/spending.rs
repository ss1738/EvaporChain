//! Spending limits and address allowlists/blocklists.
//!
//! Protects wallets from accidental overspending or sends to untrusted
//! addresses. Policies are persisted to JSON and checked before every
//! transaction submission.
//!
//! # Features
//!
//! - Per-transaction spending cap
//! - Daily rolling spending limit with automatic reset
//! - Address allowlist (only send to known addresses)
//! - Address blocklist (prevent sends to specific addresses)
//! - Configurable enforcement (enforce / warn / disabled)

use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum SpendingError {
    #[error("transaction exceeds per-tx limit: {amount} > {limit} EVAP")]
    PerTxLimitExceeded { amount: u64, limit: u64 },

    #[error("daily spending limit exceeded: spent {spent} + {amount} > {limit} EVAP")]
    DailyLimitExceeded { spent: u64, amount: u64, limit: u64 },

    #[error("address not on allowlist: {0}")]
    NotOnAllowlist(String),

    #[error("address is blocklisted: {0}")]
    Blocklisted(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

// ──────────────────────────── Policy ─────────────────────────────────────

/// Enforcement mode for spending policies.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EnforcementMode {
    /// Block transactions that violate policy.
    Enforce,
    /// Warn but allow transactions.
    Warn,
    /// Disabled — no checks.
    #[default]
    Disabled,
}

/// Spending policy configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpendingPolicy {
    /// Per-transaction maximum (0 = unlimited).
    #[serde(default)]
    pub per_tx_limit: u64,

    /// Daily rolling limit (0 = unlimited).
    #[serde(default)]
    pub daily_limit: u64,

    /// Enforcement mode.
    #[serde(default)]
    pub mode: EnforcementMode,

    /// Addresses allowed as recipients (empty = all allowed).
    #[serde(default)]
    pub allowlist: Vec<String>,

    /// Addresses blocked as recipients.
    #[serde(default)]
    pub blocklist: Vec<String>,

    /// Daily spending tracker: (date_string, amount_spent).
    #[serde(default)]
    pub daily_spent: DailyTracker,
}

/// Tracks daily spending with automatic date rollover.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyTracker {
    /// Date string (YYYY-MM-DD) of the current tracking period.
    pub date: String,
    /// Total amount spent today.
    pub spent: u64,
}

impl Default for DailyTracker {
    fn default() -> Self {
        Self {
            date: today_str(),
            spent: 0,
        }
    }
}

impl Default for SpendingPolicy {
    fn default() -> Self {
        Self {
            per_tx_limit: 0,
            daily_limit: 0,
            mode: EnforcementMode::Disabled,
            allowlist: Vec::new(),
            blocklist: Vec::new(),
            daily_spent: DailyTracker::default(),
        }
    }
}

impl SpendingPolicy {
    /// Create a new policy with limits.
    pub fn new(per_tx_limit: u64, daily_limit: u64, mode: EnforcementMode) -> Self {
        Self {
            per_tx_limit,
            daily_limit,
            mode,
            allowlist: Vec::new(),
            blocklist: Vec::new(),
            daily_spent: DailyTracker::default(),
        }
    }

    /// Load policy from a JSON file.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, SpendingError> {
        let data = std::fs::read_to_string(path)?;
        let policy: SpendingPolicy = serde_json::from_str(&data)?;
        Ok(policy)
    }

    /// Save policy to a JSON file.
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), SpendingError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Check if a transfer is allowed under this policy.
    /// Returns Ok(warnings) if allowed, Err if blocked.
    pub fn check_transfer(&mut self, to: &str, amount: u64) -> Result<Vec<String>, SpendingError> {
        if self.mode == EnforcementMode::Disabled {
            return Ok(Vec::new());
        }

        let mut warnings = Vec::new();

        // Per-tx limit
        if self.per_tx_limit > 0 && amount > self.per_tx_limit {
            let err = SpendingError::PerTxLimitExceeded {
                amount,
                limit: self.per_tx_limit,
            };
            if self.mode == EnforcementMode::Enforce {
                return Err(err);
            }
            warnings.push(format!("WARNING: {}", err));
        }

        // Daily limit
        self.rollover_if_needed();
        if self.daily_limit > 0 && self.daily_spent.spent + amount > self.daily_limit {
            let err = SpendingError::DailyLimitExceeded {
                spent: self.daily_spent.spent,
                amount,
                limit: self.daily_limit,
            };
            if self.mode == EnforcementMode::Enforce {
                return Err(err);
            }
            warnings.push(format!("WARNING: {}", err));
        }

        // Blocklist check
        let to_lower = to.to_lowercase();
        if self.blocklist.iter().any(|a| a.to_lowercase() == to_lower) {
            let err = SpendingError::Blocklisted(to.to_string());
            if self.mode == EnforcementMode::Enforce {
                return Err(err);
            }
            warnings.push(format!("WARNING: {}", err));
        }

        // Allowlist check (only if allowlist is non-empty)
        if !self.allowlist.is_empty()
            && !self.allowlist.iter().any(|a| a.to_lowercase() == to_lower)
        {
            let err = SpendingError::NotOnAllowlist(to.to_string());
            if self.mode == EnforcementMode::Enforce {
                return Err(err);
            }
            warnings.push(format!("WARNING: {}", err));
        }

        Ok(warnings)
    }

    /// Record a successful transaction (updates daily tracker).
    pub fn record_spend(&mut self, amount: u64) {
        self.rollover_if_needed();
        self.daily_spent.spent += amount;
    }

    /// Get remaining daily allowance.
    pub fn daily_remaining(&mut self) -> Option<u64> {
        if self.daily_limit == 0 {
            return None;
        }
        self.rollover_if_needed();
        Some(self.daily_limit.saturating_sub(self.daily_spent.spent))
    }

    /// Add an address to the allowlist.
    pub fn add_to_allowlist(&mut self, address: &str) {
        let addr = address.to_lowercase();
        if !self.allowlist.iter().any(|a| a.to_lowercase() == addr) {
            self.allowlist.push(address.to_string());
        }
    }

    /// Remove an address from the allowlist.
    pub fn remove_from_allowlist(&mut self, address: &str) {
        let addr = address.to_lowercase();
        self.allowlist.retain(|a| a.to_lowercase() != addr);
    }

    /// Add an address to the blocklist.
    pub fn add_to_blocklist(&mut self, address: &str) {
        let addr = address.to_lowercase();
        if !self.blocklist.iter().any(|a| a.to_lowercase() == addr) {
            self.blocklist.push(address.to_string());
        }
    }

    /// Remove an address from the blocklist.
    pub fn remove_from_blocklist(&mut self, address: &str) {
        let addr = address.to_lowercase();
        self.blocklist.retain(|a| a.to_lowercase() != addr);
    }

    /// Check if policy is active (not disabled).
    pub fn is_active(&self) -> bool {
        self.mode != EnforcementMode::Disabled
    }

    /// Reset daily spending counter.
    pub fn reset_daily(&mut self) {
        self.daily_spent = DailyTracker::default();
    }

    /// Rollover daily tracker if date has changed.
    fn rollover_if_needed(&mut self) {
        let today = today_str();
        if self.daily_spent.date != today {
            self.daily_spent.date = today;
            self.daily_spent.spent = 0;
        }
    }
}

// ──────────────────────────── Helpers ────────────────────────────────────

fn today_str() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

/// Default path for spending policy file.
pub fn default_policy_path() -> std::path::PathBuf {
    crate::config::default_data_dir().join("spending_policy.json")
}

// ──────────────────────────── Tests ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_policy() -> SpendingPolicy {
        SpendingPolicy::new(10_000, 50_000, EnforcementMode::Enforce)
    }

    #[test]
    fn test_default_policy_disabled() {
        let policy = SpendingPolicy::default();
        assert_eq!(policy.mode, EnforcementMode::Disabled);
        assert_eq!(policy.per_tx_limit, 0);
        assert_eq!(policy.daily_limit, 0);
    }

    #[test]
    fn test_check_transfer_disabled_always_ok() {
        let mut policy = SpendingPolicy::default();
        let result = policy.check_transfer("0xabc", 999_999_999);
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_per_tx_limit_enforced() {
        let mut policy = make_policy();
        // Under limit
        assert!(policy.check_transfer("0xabc", 5_000).is_ok());
        // Over limit
        let err = policy.check_transfer("0xabc", 15_000).unwrap_err();
        assert!(matches!(err, SpendingError::PerTxLimitExceeded { .. }));
    }

    #[test]
    fn test_per_tx_limit_warn_mode() {
        let mut policy = SpendingPolicy::new(10_000, 0, EnforcementMode::Warn);
        let result = policy.check_transfer("0xabc", 15_000).unwrap();
        assert!(!result.is_empty());
        assert!(result[0].contains("WARNING"));
    }

    #[test]
    fn test_daily_limit_enforced() {
        let mut policy = SpendingPolicy::new(0, 50_000, EnforcementMode::Enforce);
        // First spend
        policy.record_spend(30_000);
        // Second spend that would exceed daily (30k + 25k > 50k)
        let err = policy.check_transfer("0xabc", 25_000).unwrap_err();
        assert!(matches!(err, SpendingError::DailyLimitExceeded { .. }));
    }

    #[test]
    fn test_daily_remaining() {
        let mut policy = make_policy();
        assert_eq!(policy.daily_remaining(), Some(50_000));
        policy.record_spend(20_000);
        assert_eq!(policy.daily_remaining(), Some(30_000));
    }

    #[test]
    fn test_daily_remaining_unlimited() {
        let mut policy = SpendingPolicy::default();
        assert_eq!(policy.daily_remaining(), None);
    }

    #[test]
    fn test_blocklist() {
        let mut policy = make_policy();
        policy.add_to_blocklist("0xbad");

        let err = policy.check_transfer("0xbad", 1_000).unwrap_err();
        assert!(matches!(err, SpendingError::Blocklisted(_)));
    }

    #[test]
    fn test_blocklist_case_insensitive() {
        let mut policy = make_policy();
        policy.add_to_blocklist("0xABC");

        let err = policy.check_transfer("0xabc", 1_000).unwrap_err();
        assert!(matches!(err, SpendingError::Blocklisted(_)));
    }

    #[test]
    fn test_allowlist_enforced() {
        let mut policy = make_policy();
        policy.add_to_allowlist("0xgood");

        // Allowed address
        assert!(policy.check_transfer("0xgood", 1_000).is_ok());
        // Not on allowlist
        let err = policy.check_transfer("0xother", 1_000).unwrap_err();
        assert!(matches!(err, SpendingError::NotOnAllowlist(_)));
    }

    #[test]
    fn test_allowlist_empty_means_all_allowed() {
        let mut policy = make_policy();
        assert!(policy.allowlist.is_empty());
        assert!(policy.check_transfer("0xanyone", 1_000).is_ok());
    }

    #[test]
    fn test_add_remove_allowlist() {
        let mut policy = SpendingPolicy::default();
        policy.add_to_allowlist("0xabc");
        assert_eq!(policy.allowlist.len(), 1);
        // No duplicates
        policy.add_to_allowlist("0xabc");
        assert_eq!(policy.allowlist.len(), 1);
        // Remove
        policy.remove_from_allowlist("0xabc");
        assert!(policy.allowlist.is_empty());
    }

    #[test]
    fn test_add_remove_blocklist() {
        let mut policy = SpendingPolicy::default();
        policy.add_to_blocklist("0xbad");
        assert_eq!(policy.blocklist.len(), 1);
        policy.add_to_blocklist("0xbad");
        assert_eq!(policy.blocklist.len(), 1);
        policy.remove_from_blocklist("0xbad");
        assert!(policy.blocklist.is_empty());
    }

    #[test]
    fn test_record_spend() {
        let mut policy = make_policy();
        policy.record_spend(5_000);
        assert_eq!(policy.daily_spent.spent, 5_000);
        policy.record_spend(3_000);
        assert_eq!(policy.daily_spent.spent, 8_000);
    }

    #[test]
    fn test_reset_daily() {
        let mut policy = make_policy();
        policy.record_spend(10_000);
        assert_eq!(policy.daily_spent.spent, 10_000);
        policy.reset_daily();
        assert_eq!(policy.daily_spent.spent, 0);
    }

    #[test]
    fn test_is_active() {
        let mut policy = SpendingPolicy::default();
        assert!(!policy.is_active());
        policy.mode = EnforcementMode::Enforce;
        assert!(policy.is_active());
        policy.mode = EnforcementMode::Warn;
        assert!(policy.is_active());
    }

    #[test]
    fn test_json_roundtrip() {
        let mut policy = make_policy();
        policy.add_to_allowlist("0xfriend");
        policy.add_to_blocklist("0xenemy");
        policy.record_spend(5_000);

        let json = serde_json::to_string_pretty(&policy).unwrap();
        let loaded: SpendingPolicy = serde_json::from_str(&json).unwrap();

        assert_eq!(loaded.per_tx_limit, 10_000);
        assert_eq!(loaded.daily_limit, 50_000);
        assert_eq!(loaded.mode, EnforcementMode::Enforce);
        assert_eq!(loaded.allowlist.len(), 1);
        assert_eq!(loaded.blocklist.len(), 1);
        assert_eq!(loaded.daily_spent.spent, 5_000);
    }

    #[test]
    fn test_file_save_and_load() {
        let dir = std::env::temp_dir().join("evaporchain_spending_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("policy.json");

        let mut policy = make_policy();
        policy.add_to_blocklist("0xbad");
        policy.save(&path).unwrap();

        let loaded = SpendingPolicy::load(&path).unwrap();
        assert_eq!(loaded.per_tx_limit, 10_000);
        assert_eq!(loaded.blocklist.len(), 1);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_enforcement_mode_serialization() {
        let json = serde_json::to_string(&EnforcementMode::Enforce).unwrap();
        assert_eq!(json, "\"enforce\"");
        let json = serde_json::to_string(&EnforcementMode::Warn).unwrap();
        assert_eq!(json, "\"warn\"");
        let json = serde_json::to_string(&EnforcementMode::Disabled).unwrap();
        assert_eq!(json, "\"disabled\"");
    }

    #[test]
    fn test_multiple_violations_warn_mode() {
        let mut policy = SpendingPolicy::new(1_000, 5_000, EnforcementMode::Warn);
        policy.add_to_blocklist("0xbad");
        policy.record_spend(4_500);

        // Over per-tx, over daily, and blocklisted — should get 3 warnings
        let warnings = policy.check_transfer("0xbad", 2_000).unwrap();
        assert!(warnings.len() >= 2); // at least per-tx + daily exceeded
    }
}
