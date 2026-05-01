//! Watchtower — background monitoring daemon for energy, balances, bridge status.
//!
//! Runs periodic checks and fires notifications/actions when conditions are met.
//! Designed for headless / daemon mode: poll at intervals, check conditions,
//! act on triggers.

use std::path::Path;

use serde::{Deserialize, Serialize};

// ──────────────────────────── Types ──────────────────────────────────────

#[derive(Debug, Clone, thiserror::Error)]
pub enum WatchtowerError {
    #[error("watch not found: {0}")]
    NotFound(String),
    #[error("duplicate watch: {0}")]
    Duplicate(String),
    #[error("invalid condition: {0}")]
    InvalidCondition(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("json error: {0}")]
    Json(String),
}

impl From<std::io::Error> for WatchtowerError {
    fn from(e: std::io::Error) -> Self {
        WatchtowerError::Io(e.to_string())
    }
}
impl From<serde_json::Error> for WatchtowerError {
    fn from(e: serde_json::Error) -> Self {
        WatchtowerError::Json(e.to_string())
    }
}

/// What the watchtower monitors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatchTarget {
    /// Monitor balance of an address.
    Balance { address: String },
    /// Monitor energy of a specific object.
    Energy { object_id: String },
    /// Monitor a bridge transfer status.
    BridgeTransfer { transfer_id: String },
    /// Monitor a staking pool for reward availability.
    StakingReward { pool_id: String },
    /// Monitor block height / chain liveness.
    ChainHealth,
}

impl WatchTarget {
    pub fn label(&self) -> String {
        match self {
            WatchTarget::Balance { address } => {
                format!("Balance({})", &address[..10.min(address.len())])
            }
            WatchTarget::Energy { object_id } => format!("Energy({})", object_id),
            WatchTarget::BridgeTransfer { transfer_id } => {
                format!("Bridge({})", &transfer_id[..10.min(transfer_id.len())])
            }
            WatchTarget::StakingReward { pool_id } => format!("Staking({})", pool_id),
            WatchTarget::ChainHealth => "ChainHealth".to_string(),
        }
    }
}

/// Condition that triggers an alert.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Condition {
    /// Value drops below threshold.
    Below(f64),
    /// Value rises above threshold.
    Above(f64),
    /// Value changes by more than percentage.
    ChangeBy(f64),
    /// Status matches expected string.
    StatusEquals(String),
    /// Any change from last check.
    AnyChange,
}

impl Condition {
    pub fn label(&self) -> String {
        match self {
            Condition::Below(v) => format!("< {}", v),
            Condition::Above(v) => format!("> {}", v),
            Condition::ChangeBy(pct) => format!("changes by {}%", pct),
            Condition::StatusEquals(s) => format!("status = {}", s),
            Condition::AnyChange => "any change".to_string(),
        }
    }

    /// Check if condition is met given current and previous values.
    pub fn check(&self, current: f64, previous: Option<f64>) -> bool {
        match self {
            Condition::Below(threshold) => current < *threshold,
            Condition::Above(threshold) => current > *threshold,
            Condition::ChangeBy(pct) => {
                if let Some(prev) = previous {
                    if prev == 0.0 {
                        current != 0.0
                    } else {
                        let change = ((current - prev) / prev).abs() * 100.0;
                        change >= *pct
                    }
                } else {
                    false
                }
            }
            Condition::StatusEquals(_) => false, // Status checks use check_status
            Condition::AnyChange => {
                previous.is_none_or(|prev| (current - prev).abs() > f64::EPSILON)
            }
        }
    }

    /// Check status-based condition.
    pub fn check_status(&self, current_status: &str) -> bool {
        match self {
            Condition::StatusEquals(expected) => {
                current_status.to_lowercase() == expected.to_lowercase()
            }
            _ => false,
        }
    }
}

/// What action to take when triggered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatchAction {
    /// Fire a notification.
    Notify,
    /// Log to file.
    Log { path: String },
    /// Execute a shell command.
    Shell { command: String },
    /// Auto-refresh an object's energy.
    AutoRefresh { energy: u64 },
}

impl WatchAction {
    pub fn label(&self) -> String {
        match self {
            WatchAction::Notify => "Notify".to_string(),
            WatchAction::Log { path } => format!("Log({})", path),
            WatchAction::Shell { command } => {
                format!("Shell({})", &command[..20.min(command.len())])
            }
            WatchAction::AutoRefresh { energy } => format!("AutoRefresh({})", energy),
        }
    }
}

/// A single watch entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Watch {
    /// Unique watch ID.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// What to monitor.
    pub target: WatchTarget,
    /// When to trigger.
    pub condition: Condition,
    /// What to do.
    pub action: WatchAction,
    /// Whether active.
    pub enabled: bool,
    /// Poll interval in seconds.
    pub interval_secs: u64,
    /// Last checked timestamp.
    pub last_checked: Option<String>,
    /// Last observed value.
    pub last_value: Option<f64>,
    /// Last observed status (for status-type watches).
    pub last_status: Option<String>,
    /// Number of times triggered.
    pub trigger_count: u64,
    /// Last triggered timestamp.
    pub last_triggered: Option<String>,
    /// Created at.
    pub created_at: String,
}

impl Watch {
    /// Check if this watch is due for a poll.
    pub fn is_due(&self) -> bool {
        if !self.enabled {
            return false;
        }
        match &self.last_checked {
            Some(ts) => {
                if let Ok(last) = chrono::DateTime::parse_from_rfc3339(ts) {
                    let elapsed =
                        chrono::Utc::now().signed_duration_since(last.with_timezone(&chrono::Utc));
                    elapsed.num_seconds() >= self.interval_secs as i64
                } else {
                    true
                }
            }
            None => true,
        }
    }

    /// Record a check (value-based).
    pub fn record_check(&mut self, value: f64) -> bool {
        let triggered = self.condition.check(value, self.last_value);
        self.last_checked = Some(chrono::Utc::now().to_rfc3339());
        let _prev = self.last_value;
        self.last_value = Some(value);

        if triggered {
            self.trigger_count += 1;
            self.last_triggered = Some(chrono::Utc::now().to_rfc3339());
        }
        triggered
    }

    /// Record a check (status-based).
    pub fn record_status_check(&mut self, status: &str) -> bool {
        let triggered = self.condition.check_status(status);
        self.last_checked = Some(chrono::Utc::now().to_rfc3339());
        self.last_status = Some(status.to_string());

        if triggered {
            self.trigger_count += 1;
            self.last_triggered = Some(chrono::Utc::now().to_rfc3339());
        }
        triggered
    }
}

/// A triggered alert for logging / display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alert {
    pub watch_id: String,
    pub watch_name: String,
    pub target: String,
    pub condition: String,
    pub value: Option<f64>,
    pub status: Option<String>,
    pub action: String,
    pub timestamp: String,
}

// ──────────────────────────── Watchtower ─────────────────────────────────

/// The watchtower daemon state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Watchtower {
    pub watches: Vec<Watch>,
    pub alert_history: Vec<Alert>,
    /// Max alert history to retain.
    pub max_alerts: usize,
}

impl Watchtower {
    pub fn new() -> Self {
        Self {
            watches: Vec::new(),
            alert_history: Vec::new(),
            max_alerts: 1000,
        }
    }

    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, WatchtowerError> {
        let data = std::fs::read_to_string(path)?;
        let wt: Watchtower = serde_json::from_str(&data)?;
        Ok(wt)
    }

    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), WatchtowerError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Add a new watch.
    pub fn add_watch(
        &mut self,
        name: &str,
        target: WatchTarget,
        condition: Condition,
        action: WatchAction,
        interval_secs: u64,
    ) -> Result<&Watch, WatchtowerError> {
        if self.watches.iter().any(|w| w.name == name) {
            return Err(WatchtowerError::Duplicate(name.to_string()));
        }

        let id = format!("w_{}", chrono::Utc::now().timestamp_millis());
        let watch = Watch {
            id,
            name: name.to_string(),
            target,
            condition,
            action,
            enabled: true,
            interval_secs,
            last_checked: None,
            last_value: None,
            last_status: None,
            trigger_count: 0,
            last_triggered: None,
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        self.watches.push(watch);
        Ok(self.watches.last().unwrap())
    }

    /// Remove a watch by name.
    pub fn remove_watch(&mut self, name: &str) -> Result<(), WatchtowerError> {
        let idx = self
            .watches
            .iter()
            .position(|w| w.name == name)
            .ok_or_else(|| WatchtowerError::NotFound(name.to_string()))?;
        self.watches.remove(idx);
        Ok(())
    }

    /// Enable a watch.
    pub fn enable(&mut self, name: &str) -> Result<(), WatchtowerError> {
        let w = self
            .watches
            .iter_mut()
            .find(|w| w.name == name)
            .ok_or_else(|| WatchtowerError::NotFound(name.to_string()))?;
        w.enabled = true;
        Ok(())
    }

    /// Disable a watch.
    pub fn disable(&mut self, name: &str) -> Result<(), WatchtowerError> {
        let w = self
            .watches
            .iter_mut()
            .find(|w| w.name == name)
            .ok_or_else(|| WatchtowerError::NotFound(name.to_string()))?;
        w.enabled = false;
        Ok(())
    }

    /// Get a watch by name.
    pub fn get(&self, name: &str) -> Option<&Watch> {
        self.watches.iter().find(|w| w.name == name)
    }

    /// Get mutable.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut Watch> {
        self.watches.iter_mut().find(|w| w.name == name)
    }

    /// Get all watches due for polling.
    pub fn due_watches(&self) -> Vec<&Watch> {
        self.watches.iter().filter(|w| w.is_due()).collect()
    }

    /// Process a value-based check result, return alert if triggered.
    pub fn check_value(&mut self, name: &str, value: f64) -> Option<Alert> {
        let watch = self.watches.iter_mut().find(|w| w.name == name)?;
        let triggered = watch.record_check(value);
        if triggered {
            let alert = Alert {
                watch_id: watch.id.clone(),
                watch_name: watch.name.clone(),
                target: watch.target.label(),
                condition: watch.condition.label(),
                value: Some(value),
                status: None,
                action: watch.action.label(),
                timestamp: chrono::Utc::now().to_rfc3339(),
            };
            self.alert_history.push(alert.clone());
            if self.alert_history.len() > self.max_alerts {
                self.alert_history.remove(0);
            }
            Some(alert)
        } else {
            None
        }
    }

    /// Process a status-based check result, return alert if triggered.
    pub fn check_status(&mut self, name: &str, status: &str) -> Option<Alert> {
        let watch = self.watches.iter_mut().find(|w| w.name == name)?;
        let triggered = watch.record_status_check(status);
        if triggered {
            let alert = Alert {
                watch_id: watch.id.clone(),
                watch_name: watch.name.clone(),
                target: watch.target.label(),
                condition: watch.condition.label(),
                value: None,
                status: Some(status.to_string()),
                action: watch.action.label(),
                timestamp: chrono::Utc::now().to_rfc3339(),
            };
            self.alert_history.push(alert.clone());
            if self.alert_history.len() > self.max_alerts {
                self.alert_history.remove(0);
            }
            Some(alert)
        } else {
            None
        }
    }

    /// List all watches.
    pub fn list(&self) -> &[Watch] {
        &self.watches
    }

    /// List active watches only.
    pub fn active(&self) -> Vec<&Watch> {
        self.watches.iter().filter(|w| w.enabled).collect()
    }

    /// Recent alerts.
    pub fn recent_alerts(&self, count: usize) -> Vec<&Alert> {
        self.alert_history.iter().rev().take(count).collect()
    }

    /// Total watch count.
    pub fn watch_count(&self) -> usize {
        self.watches.len()
    }

    /// Total alerts fired.
    pub fn alert_count(&self) -> usize {
        self.alert_history.len()
    }

    /// Clear alert history.
    pub fn clear_alerts(&mut self) {
        self.alert_history.clear();
    }
}

impl Default for Watchtower {
    fn default() -> Self {
        Self::new()
    }
}

/// Default path.
pub fn default_watchtower_path() -> std::path::PathBuf {
    crate::config::default_data_dir().join("watchtower.json")
}

// ──────────────────────────── Tests ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_wt() -> Watchtower {
        let mut wt = Watchtower::new();
        wt.add_watch(
            "low-balance",
            WatchTarget::Balance {
                address: "0xmyaddr".into(),
            },
            Condition::Below(1000.0),
            WatchAction::Notify,
            60,
        )
        .unwrap();
        wt.add_watch(
            "energy-critical",
            WatchTarget::Energy {
                object_id: "obj_42".into(),
            },
            Condition::Below(10.0),
            WatchAction::AutoRefresh { energy: 5000 },
            120,
        )
        .unwrap();
        wt.add_watch(
            "bridge-done",
            WatchTarget::BridgeTransfer {
                transfer_id: "bt_001".into(),
            },
            Condition::StatusEquals("completed".into()),
            WatchAction::Notify,
            30,
        )
        .unwrap();
        wt
    }

    #[test]
    fn test_add_watch() {
        let wt = make_wt();
        assert_eq!(wt.watch_count(), 3);
    }

    #[test]
    fn test_duplicate_rejected() {
        let mut wt = make_wt();
        let err = wt.add_watch(
            "low-balance",
            WatchTarget::ChainHealth,
            Condition::AnyChange,
            WatchAction::Notify,
            60,
        );
        assert!(err.is_err());
    }

    #[test]
    fn test_remove_watch() {
        let mut wt = make_wt();
        wt.remove_watch("low-balance").unwrap();
        assert_eq!(wt.watch_count(), 2);
        assert!(wt.get("low-balance").is_none());
    }

    #[test]
    fn test_remove_nonexistent() {
        let mut wt = make_wt();
        assert!(wt.remove_watch("nope").is_err());
    }

    #[test]
    fn test_enable_disable() {
        let mut wt = make_wt();
        wt.disable("low-balance").unwrap();
        assert!(!wt.get("low-balance").unwrap().enabled);
        wt.enable("low-balance").unwrap();
        assert!(wt.get("low-balance").unwrap().enabled);
    }

    #[test]
    fn test_check_value_triggers() {
        let mut wt = make_wt();
        let alert = wt.check_value("low-balance", 500.0);
        assert!(alert.is_some());
        let a = alert.unwrap();
        assert_eq!(a.watch_name, "low-balance");
        assert_eq!(a.value, Some(500.0));
        assert_eq!(wt.alert_count(), 1);
    }

    #[test]
    fn test_check_value_no_trigger() {
        let mut wt = make_wt();
        let alert = wt.check_value("low-balance", 5000.0);
        assert!(alert.is_none());
        assert_eq!(wt.alert_count(), 0);
    }

    #[test]
    fn test_check_status_triggers() {
        let mut wt = make_wt();
        let alert = wt.check_status("bridge-done", "completed");
        assert!(alert.is_some());
        let a = alert.unwrap();
        assert_eq!(a.status.as_deref(), Some("completed"));
    }

    #[test]
    fn test_check_status_no_trigger() {
        let mut wt = make_wt();
        let alert = wt.check_status("bridge-done", "bridging");
        assert!(alert.is_none());
    }

    #[test]
    fn test_trigger_count() {
        let mut wt = make_wt();
        wt.check_value("energy-critical", 5.0);
        wt.check_value("energy-critical", 3.0);
        wt.check_value("energy-critical", 50.0); // not triggered
        let w = wt.get("energy-critical").unwrap();
        assert_eq!(w.trigger_count, 2);
    }

    #[test]
    fn test_condition_below() {
        assert!(Condition::Below(100.0).check(50.0, None));
        assert!(!Condition::Below(100.0).check(200.0, None));
    }

    #[test]
    fn test_condition_above() {
        assert!(Condition::Above(100.0).check(200.0, None));
        assert!(!Condition::Above(100.0).check(50.0, None));
    }

    #[test]
    fn test_condition_change_by() {
        let cond = Condition::ChangeBy(10.0);
        assert!(cond.check(120.0, Some(100.0))); // 20% change
        assert!(!cond.check(105.0, Some(100.0))); // 5% change
        assert!(!cond.check(100.0, None)); // no previous
    }

    #[test]
    fn test_condition_any_change() {
        let cond = Condition::AnyChange;
        assert!(cond.check(100.0, Some(99.0)));
        assert!(!cond.check(100.0, Some(100.0)));
        assert!(cond.check(100.0, None)); // first check = changed
    }

    #[test]
    fn test_condition_status_equals() {
        let cond = Condition::StatusEquals("completed".into());
        assert!(cond.check_status("completed"));
        assert!(cond.check_status("Completed")); // case-insensitive
        assert!(!cond.check_status("bridging"));
    }

    #[test]
    fn test_due_watches_initial() {
        let wt = make_wt();
        // All watches are due initially (never checked)
        assert_eq!(wt.due_watches().len(), 3);
    }

    #[test]
    fn test_due_watches_after_check() {
        let mut wt = make_wt();
        wt.check_value("low-balance", 5000.0);
        // low-balance was just checked, not due yet
        let w = wt.get("low-balance").unwrap();
        assert!(!w.is_due()); // interval is 60s, just checked
    }

    #[test]
    fn test_disabled_not_due() {
        let mut wt = make_wt();
        wt.disable("low-balance").unwrap();
        assert!(!wt.get("low-balance").unwrap().is_due());
    }

    #[test]
    fn test_active_filter() {
        let mut wt = make_wt();
        wt.disable("bridge-done").unwrap();
        let active = wt.active();
        assert_eq!(active.len(), 2);
    }

    #[test]
    fn test_recent_alerts() {
        let mut wt = make_wt();
        wt.check_value("low-balance", 500.0);
        wt.check_value("energy-critical", 5.0);
        let recent = wt.recent_alerts(10);
        assert_eq!(recent.len(), 2);
        // Most recent first
        assert_eq!(recent[0].watch_name, "energy-critical");
    }

    #[test]
    fn test_clear_alerts() {
        let mut wt = make_wt();
        wt.check_value("low-balance", 500.0);
        wt.clear_alerts();
        assert_eq!(wt.alert_count(), 0);
    }

    #[test]
    fn test_watch_target_labels() {
        assert_eq!(WatchTarget::ChainHealth.label(), "ChainHealth");
        let bt = WatchTarget::Balance {
            address: "0xabcdef1234".into(),
        };
        assert!(bt.label().contains("Balance"));
    }

    #[test]
    fn test_watch_action_labels() {
        assert_eq!(WatchAction::Notify.label(), "Notify");
        assert!(WatchAction::AutoRefresh { energy: 5000 }
            .label()
            .contains("5000"));
    }

    #[test]
    fn test_persistence_roundtrip() {
        let dir = std::env::temp_dir().join("evap_wt_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("watchtower.json");

        let mut wt = make_wt();
        wt.check_value("low-balance", 500.0);
        wt.save(&path).unwrap();

        let loaded = Watchtower::load(&path).unwrap();
        assert_eq!(loaded.watch_count(), 3);
        assert_eq!(loaded.alert_count(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_alert_history_cap() {
        let mut wt = Watchtower::new();
        wt.max_alerts = 3;
        wt.add_watch(
            "test",
            WatchTarget::ChainHealth,
            Condition::AnyChange,
            WatchAction::Notify,
            1,
        )
        .unwrap();
        for i in 0..5 {
            wt.check_value("test", i as f64);
        }
        assert!(wt.alert_count() <= 3);
    }

    #[test]
    fn test_condition_change_by_from_zero() {
        let cond = Condition::ChangeBy(10.0);
        // Previous was 0, current is 100 — should trigger
        assert!(cond.check(100.0, Some(0.0)));
        // Previous was 0, current is 0 — no change
        assert!(!cond.check(0.0, Some(0.0)));
    }

    #[test]
    fn test_default_trait() {
        let wt = Watchtower::default();
        assert_eq!(wt.watch_count(), 0);
        assert_eq!(wt.max_alerts, 1000);
    }
}
