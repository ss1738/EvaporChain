// wallet/src/watchonly.rs — Watch-only account tracker
//
// Track addresses, balances, and activity without private keys.
// Useful for cold storage monitoring, whale watching, portfolio tracking.
//   - Persistent JSON store
//   - Alert system for balance changes
//   - Tagging, search, priority filtering

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

// ── Error ────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum WatchOnlyError {
    #[error("address already watched: {0}")]
    AlreadyWatched(String),
    #[error("address not found: {0}")]
    NotFound(String),
    #[error("account is disabled: {0}")]
    Disabled(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("parse error: {0}")]
    Parse(String),
}

impl From<std::io::Error> for WatchOnlyError {
    fn from(e: std::io::Error) -> Self {
        WatchOnlyError::Io(e.to_string())
    }
}
impl From<serde_json::Error> for WatchOnlyError {
    fn from(e: serde_json::Error) -> Self {
        WatchOnlyError::Parse(e.to_string())
    }
}

// ── Enums ────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertType {
    BalanceChange,
    LargeTransfer,
    InactivityWarning,
    ObjectExpiring,
    Custom(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatchPriority {
    High,
    Medium,
    Low,
}

// ── BalanceSnapshot ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BalanceSnapshot {
    pub balance: u64,
    pub recorded_at: String,
}

// ── Alert ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alert {
    pub alert_type: AlertType,
    pub address: String,
    pub message: String,
    pub created_at: String,
    pub read: bool,
}

impl Alert {
    pub fn new(alert_type: AlertType, address: &str, message: &str) -> Self {
        Self {
            alert_type,
            address: address.to_string(),
            message: message.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            read: false,
        }
    }

    pub fn mark_read(&mut self) {
        self.read = true;
    }
}

// ── WatchedAccount ───────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchedAccount {
    pub address: String,
    pub label: String,
    pub notes: String,
    pub priority: WatchPriority,
    pub added_at: String,
    pub last_activity: Option<String>,
    pub last_balance: u64,
    pub balance_history: Vec<BalanceSnapshot>,
    pub alert_threshold: u64,
    pub alerts_enabled: bool,
    pub tags: Vec<String>,
    pub active: bool,
}

impl WatchedAccount {
    pub fn new(address: &str, label: &str) -> Self {
        Self {
            address: address.to_string(),
            label: label.to_string(),
            notes: String::new(),
            priority: WatchPriority::Medium,
            added_at: chrono::Utc::now().to_rfc3339(),
            last_activity: None,
            last_balance: 0,
            balance_history: Vec::new(),
            alert_threshold: 0,
            alerts_enabled: true,
            tags: Vec::new(),
            active: true,
        }
    }

    pub fn with_priority(mut self, p: WatchPriority) -> Self {
        self.priority = p;
        self
    }

    pub fn with_threshold(mut self, t: u64) -> Self {
        self.alert_threshold = t;
        self
    }

    pub fn with_notes(mut self, n: &str) -> Self {
        self.notes = n.to_string();
        self
    }

    /// Update balance, record a snapshot, and return an Alert if the change
    /// exceeds the configured threshold.
    pub fn update_balance(&mut self, new_balance: u64) -> Option<Alert> {
        let old = self.last_balance;
        let now = chrono::Utc::now().to_rfc3339();

        self.balance_history.push(BalanceSnapshot {
            balance: new_balance,
            recorded_at: now.clone(),
        });

        // Cap history at 100 entries
        if self.balance_history.len() > 100 {
            let excess = self.balance_history.len() - 100;
            self.balance_history.drain(..excess);
        }

        self.last_balance = new_balance;
        self.last_activity = Some(now);

        // Check if an alert should fire
        if self.alerts_enabled && self.alert_threshold > 0 {
            let diff = new_balance.abs_diff(old);
            if diff >= self.alert_threshold {
                let msg = format!(
                    "Balance changed from {} to {} (delta: {})",
                    old, new_balance, diff
                );
                return Some(Alert::new(AlertType::BalanceChange, &self.address, &msg));
            }
        }

        None
    }

    /// Signed difference from previous balance (0 if no history).
    pub fn balance_change(&self) -> i64 {
        if self.balance_history.len() < 2 {
            return 0;
        }
        let prev = self.balance_history[self.balance_history.len() - 2].balance;
        self.last_balance as i64 - prev as i64
    }

    pub fn add_tag(&mut self, tag: &str) {
        let t = tag.to_string();
        if !self.tags.contains(&t) {
            self.tags.push(t);
        }
    }

    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t == tag)
    }

    pub fn disable(&mut self) {
        self.active = false;
    }

    pub fn enable(&mut self) {
        self.active = true;
    }

    /// Days since last recorded activity, or None if no activity.
    pub fn days_since_activity(&self) -> Option<u64> {
        let last = self.last_activity.as_ref()?;
        let parsed = chrono::DateTime::parse_from_rfc3339(last).ok()?;
        let duration = chrono::Utc::now().signed_duration_since(parsed);
        Some(duration.num_days().max(0) as u64)
    }
}

// ── WatchStats ───────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchStats {
    pub total_accounts: usize,
    pub active: usize,
    pub disabled: usize,
    pub total_alerts: usize,
    pub unread_alerts: usize,
    pub total_balance: u64,
}

// ── WatchStore ───────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchStore {
    pub accounts: HashMap<String, WatchedAccount>,
    pub alerts: Vec<Alert>,
    pub max_alerts: usize,
}

impl Default for WatchStore {
    fn default() -> Self {
        Self::new()
    }
}

impl WatchStore {
    pub fn new() -> Self {
        Self {
            accounts: HashMap::new(),
            alerts: Vec::new(),
            max_alerts: 500,
        }
    }

    /// Add a watched account. Fails if the address is already tracked.
    pub fn watch(&mut self, account: WatchedAccount) -> Result<(), WatchOnlyError> {
        if self.accounts.contains_key(&account.address) {
            return Err(WatchOnlyError::AlreadyWatched(account.address));
        }
        self.accounts.insert(account.address.clone(), account);
        Ok(())
    }

    /// Remove and return a watched account.
    pub fn unwatch(&mut self, address: &str) -> Result<WatchedAccount, WatchOnlyError> {
        self.accounts
            .remove(address)
            .ok_or_else(|| WatchOnlyError::NotFound(address.to_string()))
    }

    pub fn get(&self, address: &str) -> Option<&WatchedAccount> {
        self.accounts.get(address)
    }

    pub fn get_mut(&mut self, address: &str) -> Option<&mut WatchedAccount> {
        self.accounts.get_mut(address)
    }

    /// List only active accounts.
    pub fn list(&self) -> Vec<&WatchedAccount> {
        self.accounts.values().filter(|a| a.active).collect()
    }

    /// List all accounts, including disabled.
    pub fn list_all(&self) -> Vec<&WatchedAccount> {
        self.accounts.values().collect()
    }

    /// Filter active accounts by priority.
    pub fn by_priority(&self, priority: &WatchPriority) -> Vec<&WatchedAccount> {
        self.accounts
            .values()
            .filter(|a| a.active && a.priority == *priority)
            .collect()
    }

    /// Filter active accounts by tag.
    pub fn by_tag(&self, tag: &str) -> Vec<&WatchedAccount> {
        self.accounts
            .values()
            .filter(|a| a.active && a.has_tag(tag))
            .collect()
    }

    /// Case-insensitive search on address, label, and notes.
    pub fn search(&self, query: &str) -> Vec<&WatchedAccount> {
        let q = query.to_lowercase();
        self.accounts
            .values()
            .filter(|a| {
                a.address.to_lowercase().contains(&q)
                    || a.label.to_lowercase().contains(&q)
                    || a.notes.to_lowercase().contains(&q)
            })
            .collect()
    }

    /// Update balance for a watched address. Returns any generated alert.
    pub fn update_balance(
        &mut self,
        address: &str,
        new_balance: u64,
    ) -> Result<Option<Alert>, WatchOnlyError> {
        let account = self
            .accounts
            .get_mut(address)
            .ok_or_else(|| WatchOnlyError::NotFound(address.to_string()))?;

        let alert = account.update_balance(new_balance);

        if let Some(ref a) = alert {
            self.alerts.push(a.clone());
            // Prune old alerts if over limit
            if self.alerts.len() > self.max_alerts {
                let excess = self.alerts.len() - self.max_alerts;
                self.alerts.drain(..excess);
            }
        }

        Ok(alert)
    }

    pub fn unread_alerts(&self) -> Vec<&Alert> {
        self.alerts.iter().filter(|a| !a.read).collect()
    }

    pub fn alerts_for(&self, address: &str) -> Vec<&Alert> {
        self.alerts
            .iter()
            .filter(|a| a.address == address)
            .collect()
    }

    pub fn mark_all_read(&mut self) {
        for alert in &mut self.alerts {
            alert.read = true;
        }
    }

    /// Clear all alerts, returning the number removed.
    pub fn clear_alerts(&mut self) -> usize {
        let count = self.alerts.len();
        self.alerts.clear();
        count
    }

    /// Sum of last_balance across all active accounts.
    pub fn total_watched_balance(&self) -> u64 {
        self.accounts
            .values()
            .filter(|a| a.active)
            .map(|a| a.last_balance)
            .sum()
    }

    /// Accounts with no activity for at least `days` days.
    pub fn inactive_accounts(&self, days: u64) -> Vec<&WatchedAccount> {
        self.accounts
            .values()
            .filter(|a| a.active)
            .filter(|a| match a.days_since_activity() {
                Some(d) => d >= days,
                None => true, // never had activity → considered inactive
            })
            .collect()
    }

    pub fn stats(&self) -> WatchStats {
        let total_accounts = self.accounts.len();
        let active = self.accounts.values().filter(|a| a.active).count();
        WatchStats {
            total_accounts,
            active,
            disabled: total_accounts - active,
            total_alerts: self.alerts.len(),
            unread_alerts: self.alerts.iter().filter(|a| !a.read).count(),
            total_balance: self.total_watched_balance(),
        }
    }

    // ── Persistence ──────────────────────────────────────────

    pub fn save(&self, path: &Path) -> Result<(), WatchOnlyError> {
        let json =
            serde_json::to_string_pretty(self).map_err(|e| WatchOnlyError::Parse(e.to_string()))?;
        std::fs::write(path, json).map_err(|e| WatchOnlyError::Io(e.to_string()))
    }

    pub fn load(path: &Path) -> Result<Self, WatchOnlyError> {
        let data = std::fs::read_to_string(path).map_err(|e| WatchOnlyError::Io(e.to_string()))?;
        serde_json::from_str(&data).map_err(|e| WatchOnlyError::Parse(e.to_string()))
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ── Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "watchonly_test_{}_{}.json",
            name,
            std::process::id()
        ))
    }

    fn make_account(addr: &str, label: &str) -> WatchedAccount {
        WatchedAccount::new(addr, label)
    }

    #[test]
    fn test_watch_and_get() {
        let mut store = WatchStore::new();
        let acct = make_account("evap1aaa", "Cold Wallet");
        store.watch(acct).unwrap();
        let got = store.get("evap1aaa").unwrap();
        assert_eq!(got.label, "Cold Wallet");
        assert_eq!(got.priority, WatchPriority::Medium);
        assert!(got.active);
    }

    #[test]
    fn test_watch_duplicate_rejected() {
        let mut store = WatchStore::new();
        store.watch(make_account("evap1aaa", "A")).unwrap();
        let err = store.watch(make_account("evap1aaa", "B")).unwrap_err();
        assert!(matches!(err, WatchOnlyError::AlreadyWatched(_)));
    }

    #[test]
    fn test_unwatch() {
        let mut store = WatchStore::new();
        store.watch(make_account("evap1aaa", "A")).unwrap();
        let removed = store.unwatch("evap1aaa").unwrap();
        assert_eq!(removed.address, "evap1aaa");
        assert!(store.get("evap1aaa").is_none());
    }

    #[test]
    fn test_unwatch_not_found() {
        let mut store = WatchStore::new();
        let err = store.unwatch("evap1zzz").unwrap_err();
        assert!(matches!(err, WatchOnlyError::NotFound(_)));
    }

    #[test]
    fn test_update_balance_no_alert() {
        let mut store = WatchStore::new();
        let acct = make_account("evap1aaa", "A").with_threshold(1000);
        store.watch(acct).unwrap();
        // Change from 0 to 500 — below threshold of 1000
        let alert = store.update_balance("evap1aaa", 500).unwrap();
        assert!(alert.is_none());
        assert_eq!(store.get("evap1aaa").unwrap().last_balance, 500);
    }

    #[test]
    fn test_update_balance_with_alert() {
        let mut store = WatchStore::new();
        let acct = make_account("evap1aaa", "A").with_threshold(100);
        store.watch(acct).unwrap();
        // Change from 0 to 5000 — exceeds threshold of 100
        let alert = store.update_balance("evap1aaa", 5000).unwrap();
        assert!(alert.is_some());
        let a = alert.unwrap();
        assert_eq!(a.alert_type, AlertType::BalanceChange);
        assert_eq!(a.address, "evap1aaa");
        assert!(!a.read);
        // Alert should be stored
        assert_eq!(store.alerts.len(), 1);
    }

    #[test]
    fn test_balance_change() {
        let mut acct = make_account("evap1aaa", "A");
        acct.update_balance(1000);
        assert_eq!(acct.balance_change(), 0); // only 1 entry, no previous
        acct.update_balance(1500);
        assert_eq!(acct.balance_change(), 500);
        acct.update_balance(1200);
        assert_eq!(acct.balance_change(), -300);
    }

    #[test]
    fn test_balance_history_capped() {
        let mut acct = make_account("evap1aaa", "A");
        for i in 0..=120 {
            acct.update_balance(i as u64);
        }
        assert_eq!(acct.balance_history.len(), 100);
        // Oldest should have been pruned; first entry should be 21
        assert_eq!(acct.balance_history[0].balance, 21);
    }

    #[test]
    fn test_disable_enable() {
        let mut acct = make_account("evap1aaa", "A");
        assert!(acct.active);
        acct.disable();
        assert!(!acct.active);
        acct.enable();
        assert!(acct.active);
    }

    #[test]
    fn test_list_active_only() {
        let mut store = WatchStore::new();
        store.watch(make_account("evap1aaa", "A")).unwrap();
        store.watch(make_account("evap1bbb", "B")).unwrap();
        store.get_mut("evap1bbb").unwrap().disable();
        let active = store.list();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].address, "evap1aaa");
    }

    #[test]
    fn test_list_all() {
        let mut store = WatchStore::new();
        store.watch(make_account("evap1aaa", "A")).unwrap();
        store.watch(make_account("evap1bbb", "B")).unwrap();
        store.get_mut("evap1bbb").unwrap().disable();
        assert_eq!(store.list_all().len(), 2);
    }

    #[test]
    fn test_by_priority() {
        let mut store = WatchStore::new();
        store
            .watch(make_account("evap1aaa", "A").with_priority(WatchPriority::High))
            .unwrap();
        store
            .watch(make_account("evap1bbb", "B").with_priority(WatchPriority::Low))
            .unwrap();
        store
            .watch(make_account("evap1ccc", "C").with_priority(WatchPriority::High))
            .unwrap();
        let highs = store.by_priority(&WatchPriority::High);
        assert_eq!(highs.len(), 2);
        let lows = store.by_priority(&WatchPriority::Low);
        assert_eq!(lows.len(), 1);
    }

    #[test]
    fn test_by_tag() {
        let mut store = WatchStore::new();
        let mut a = make_account("evap1aaa", "A");
        a.add_tag("whale");
        a.add_tag("defi");
        store.watch(a).unwrap();

        let mut b = make_account("evap1bbb", "B");
        b.add_tag("whale");
        store.watch(b).unwrap();

        store.watch(make_account("evap1ccc", "C")).unwrap();

        assert_eq!(store.by_tag("whale").len(), 2);
        assert_eq!(store.by_tag("defi").len(), 1);
        assert_eq!(store.by_tag("nft").len(), 0);
    }

    #[test]
    fn test_search() {
        let mut store = WatchStore::new();
        store
            .watch(make_account("evap1aaa", "Cold Wallet").with_notes("main cold storage"))
            .unwrap();
        store.watch(make_account("evap1bbb", "Hot Wallet")).unwrap();

        let results = store.search("cold");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].address, "evap1aaa");

        // Search by address fragment
        let results = store.search("evap1b");
        assert_eq!(results.len(), 1);

        // Case insensitive
        let results = store.search("HOT");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_unread_alerts() {
        let mut store = WatchStore::new();
        store
            .watch(make_account("evap1aaa", "A").with_threshold(10))
            .unwrap();
        store.update_balance("evap1aaa", 100).unwrap();
        assert_eq!(store.unread_alerts().len(), 1);
        store.alerts[0].mark_read();
        assert_eq!(store.unread_alerts().len(), 0);
    }

    #[test]
    fn test_mark_all_read() {
        let mut store = WatchStore::new();
        store
            .watch(make_account("evap1aaa", "A").with_threshold(10))
            .unwrap();
        store.update_balance("evap1aaa", 100).unwrap();
        store.update_balance("evap1aaa", 500).unwrap();
        assert_eq!(store.unread_alerts().len(), 2);
        store.mark_all_read();
        assert_eq!(store.unread_alerts().len(), 0);
    }

    #[test]
    fn test_clear_alerts() {
        let mut store = WatchStore::new();
        store
            .watch(make_account("evap1aaa", "A").with_threshold(10))
            .unwrap();
        store.update_balance("evap1aaa", 100).unwrap();
        store.update_balance("evap1aaa", 500).unwrap();
        let cleared = store.clear_alerts();
        assert_eq!(cleared, 2);
        assert!(store.alerts.is_empty());
    }

    #[test]
    fn test_total_watched_balance() {
        let mut store = WatchStore::new();
        store.watch(make_account("evap1aaa", "A")).unwrap();
        store.watch(make_account("evap1bbb", "B")).unwrap();
        store.watch(make_account("evap1ccc", "C")).unwrap();
        store.update_balance("evap1aaa", 1000).unwrap();
        store.update_balance("evap1bbb", 2000).unwrap();
        store.update_balance("evap1ccc", 3000).unwrap();
        // Disable one — should not count
        store.get_mut("evap1ccc").unwrap().disable();
        assert_eq!(store.total_watched_balance(), 3000);
    }

    #[test]
    fn test_stats() {
        let mut store = WatchStore::new();
        store
            .watch(make_account("evap1aaa", "A").with_threshold(10))
            .unwrap();
        store.watch(make_account("evap1bbb", "B")).unwrap();
        store.get_mut("evap1bbb").unwrap().disable();
        store.update_balance("evap1aaa", 500).unwrap();

        let s = store.stats();
        assert_eq!(s.total_accounts, 2);
        assert_eq!(s.active, 1);
        assert_eq!(s.disabled, 1);
        assert_eq!(s.total_alerts, 1);
        assert_eq!(s.unread_alerts, 1);
        assert_eq!(s.total_balance, 500);
    }

    #[test]
    fn test_persistence_roundtrip() {
        let path = test_path("roundtrip");
        let mut store = WatchStore::new();
        let acct = make_account("evap1aaa", "Cold Wallet")
            .with_priority(WatchPriority::High)
            .with_threshold(100)
            .with_notes("main vault");
        store.watch(acct).unwrap();
        store.update_balance("evap1aaa", 5000).unwrap();

        store.save(&path).unwrap();
        let loaded = WatchStore::load(&path).unwrap();

        assert_eq!(loaded.accounts.len(), 1);
        let a = loaded.get("evap1aaa").unwrap();
        assert_eq!(a.label, "Cold Wallet");
        assert_eq!(a.last_balance, 5000);
        assert_eq!(a.priority, WatchPriority::High);
        assert_eq!(loaded.alerts.len(), 1);

        // Cleanup
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_inactive_accounts() {
        let mut store = WatchStore::new();
        // Account with no activity at all → considered inactive
        store.watch(make_account("evap1aaa", "A")).unwrap();
        // Account with recent activity
        store.watch(make_account("evap1bbb", "B")).unwrap();
        store.update_balance("evap1bbb", 100).unwrap();

        // 0 days threshold: accounts with no activity qualify,
        // recently active ones have 0 days since activity which is >= 0
        let inactive = store.inactive_accounts(1);
        // evap1aaa has no activity (None → inactive), evap1bbb was just updated (0 days < 1)
        assert_eq!(inactive.len(), 1);
        assert_eq!(inactive[0].address, "evap1aaa");
    }
}
