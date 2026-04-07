//! Notification system — event-driven alerts for wallet activity.
//!
//! Supports multiple notification channels and priority levels:
//! - Terminal output (colored, with severity)
//! - Log file (append-only, JSON or plaintext)
//! - Notification history (persistent, queryable)
//!
//! # Event Types
//!
//! - Energy decay warnings (object approaching evaporation)
//! - Transaction confirmations
//! - Fee target alerts (gas price dropped below threshold)
//! - Security events (login, large transfer, new device)
//! - Session expiry warnings (dApp sessions about to expire)

use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum NotificationError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

// ──────────────────────────── Types ──────────────────────────────────────

/// Notification priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    Low,
    Medium,
    High,
    Critical,
}

impl Priority {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Self::Low => "INFO",
            Self::Medium => "WARN",
            Self::High => "ALERT",
            Self::Critical => "CRIT",
        }
    }
}

/// Category of notification event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventCategory {
    EnergyDecay,
    TxConfirmed,
    TxFailed,
    FeeAlert,
    Security,
    SessionExpiry,
    System,
}

impl EventCategory {
    pub fn label(&self) -> &'static str {
        match self {
            Self::EnergyDecay => "energy_decay",
            Self::TxConfirmed => "tx_confirmed",
            Self::TxFailed => "tx_failed",
            Self::FeeAlert => "fee_alert",
            Self::Security => "security",
            Self::SessionExpiry => "session_expiry",
            Self::System => "system",
        }
    }
}

/// A single notification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    /// Unique ID.
    pub id: String,
    /// Priority level.
    pub priority: Priority,
    /// Event category.
    pub category: EventCategory,
    /// Short title.
    pub title: String,
    /// Detailed message.
    pub message: String,
    /// Whether the user has read this notification.
    pub read: bool,
    /// When the notification was created.
    pub created_at: String,
    /// Optional associated data (tx hash, object ID, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
}

/// Notification channel configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NotificationChannel {
    /// Print to terminal.
    Terminal,
    /// Append to log file.
    LogFile { path: String },
}

/// Notification preferences.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationConfig {
    /// Minimum priority to show (notifications below this are silent).
    pub min_priority: Priority,
    /// Active channels.
    pub channels: Vec<NotificationChannel>,
    /// Maximum notifications to retain in history.
    pub max_history: usize,
    /// Whether notifications are enabled.
    pub enabled: bool,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            min_priority: Priority::Low,
            channels: vec![NotificationChannel::Terminal],
            max_history: 500,
            enabled: true,
        }
    }
}

// ──────────────────────────── NotificationCenter ─────────────────────────

/// Central notification manager.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationCenter {
    pub config: NotificationConfig,
    pub history: Vec<Notification>,
    next_id: u64,
}

impl NotificationCenter {
    /// Create with default config.
    pub fn new() -> Self {
        Self {
            config: NotificationConfig::default(),
            history: Vec::new(),
            next_id: 1,
        }
    }

    /// Create with custom config.
    pub fn with_config(config: NotificationConfig) -> Self {
        Self {
            config,
            history: Vec::new(),
            next_id: 1,
        }
    }

    /// Load from file.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, NotificationError> {
        let data = std::fs::read_to_string(path)?;
        let center: NotificationCenter = serde_json::from_str(&data)?;
        Ok(center)
    }

    /// Save to file.
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), NotificationError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Send a notification.
    pub fn notify(
        &mut self,
        priority: Priority,
        category: EventCategory,
        title: &str,
        message: &str,
        reference: Option<&str>,
    ) -> &Notification {
        let notif = Notification {
            id: format!("notif_{}", self.next_id),
            priority,
            category,
            title: title.to_string(),
            message: message.to_string(),
            read: false,
            created_at: chrono::Utc::now().to_rfc3339(),
            reference: reference.map(|s| s.to_string()),
        };
        self.next_id += 1;

        // Dispatch to channels
        if self.config.enabled && priority >= self.config.min_priority {
            for channel in &self.config.channels {
                let _ = dispatch(&notif, channel);
            }
        }

        // Store in history
        self.history.push(notif);

        // Trim history
        if self.history.len() > self.config.max_history {
            let excess = self.history.len() - self.config.max_history;
            self.history.drain(..excess);
        }

        self.history.last().unwrap()
    }

    // ── Convenience methods ─────────────────────────────────────────────

    /// Energy decay warning.
    pub fn energy_warning(&mut self, object_id: &str, energy_pct: f64) -> &Notification {
        let priority = if energy_pct < 5.0 {
            Priority::Critical
        } else if energy_pct < 15.0 {
            Priority::High
        } else if energy_pct < 30.0 {
            Priority::Medium
        } else {
            Priority::Low
        };

        self.notify(
            priority,
            EventCategory::EnergyDecay,
            &format!("Object {} energy at {:.1}%", truncate(object_id), energy_pct),
            &format!(
                "Object {} has {:.1}% energy remaining. Consider refreshing to prevent evaporation.",
                object_id, energy_pct
            ),
            Some(object_id),
        )
    }

    /// Transaction confirmed.
    pub fn tx_confirmed(&mut self, tx_hash: &str, tx_type: &str) -> &Notification {
        self.notify(
            Priority::Low,
            EventCategory::TxConfirmed,
            &format!("{} confirmed", tx_type),
            &format!("Transaction {} ({}) has been confirmed on-chain.", truncate(tx_hash), tx_type),
            Some(tx_hash),
        )
    }

    /// Transaction failed.
    pub fn tx_failed(&mut self, tx_hash: &str, reason: &str) -> &Notification {
        self.notify(
            Priority::High,
            EventCategory::TxFailed,
            "Transaction failed",
            &format!("Transaction {} failed: {}", truncate(tx_hash), reason),
            Some(tx_hash),
        )
    }

    /// Fee alert triggered.
    pub fn fee_alert(&mut self, alert_name: &str, current_fee: u64) -> &Notification {
        self.notify(
            Priority::Medium,
            EventCategory::FeeAlert,
            &format!("Fee alert: {}", alert_name),
            &format!("Gas fee dropped to {} — alert '{}' triggered.", current_fee, alert_name),
            None,
        )
    }

    /// Security event.
    pub fn security_event(&mut self, title: &str, detail: &str) -> &Notification {
        self.notify(
            Priority::High,
            EventCategory::Security,
            title,
            detail,
            None,
        )
    }

    /// Session expiry warning.
    pub fn session_expiring(&mut self, session_id: &str, app_name: &str) -> &Notification {
        self.notify(
            Priority::Medium,
            EventCategory::SessionExpiry,
            &format!("dApp session expiring: {}", app_name),
            &format!("Session {} for {} is about to expire.", session_id, app_name),
            Some(session_id),
        )
    }

    // ── Query ───────────────────────────────────────────────────────────

    /// Get unread notifications.
    pub fn unread(&self) -> Vec<&Notification> {
        self.history.iter().filter(|n| !n.read).collect()
    }

    /// Get unread count.
    pub fn unread_count(&self) -> usize {
        self.history.iter().filter(|n| !n.read).count()
    }

    /// Mark a notification as read.
    pub fn mark_read(&mut self, id: &str) {
        if let Some(n) = self.history.iter_mut().find(|n| n.id == id) {
            n.read = true;
        }
    }

    /// Mark all as read.
    pub fn mark_all_read(&mut self) {
        for n in &mut self.history {
            n.read = true;
        }
    }

    /// Filter by category.
    pub fn filter_by_category(&self, category: EventCategory) -> Vec<&Notification> {
        self.history.iter().filter(|n| n.category == category).collect()
    }

    /// Filter by priority (minimum).
    pub fn filter_by_priority(&self, min: Priority) -> Vec<&Notification> {
        self.history.iter().filter(|n| n.priority >= min).collect()
    }

    /// Get recent notifications (last N).
    pub fn recent(&self, count: usize) -> Vec<&Notification> {
        self.history.iter().rev().take(count).collect()
    }

    /// Clear all history.
    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    /// Total notifications in history.
    pub fn len(&self) -> usize {
        self.history.len()
    }

    pub fn is_empty(&self) -> bool {
        self.history.is_empty()
    }
}

impl Default for NotificationCenter {
    fn default() -> Self {
        Self::new()
    }
}

// ──────────────────────────── Dispatch ────────────────────────────────────

fn dispatch(notif: &Notification, channel: &NotificationChannel) -> Result<(), NotificationError> {
    match channel {
        NotificationChannel::Terminal => {
            // Just format — actual printing happens in caller context
            Ok(())
        }
        NotificationChannel::LogFile { path } => {
            use std::io::Write;
            let p = Path::new(path);
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)?;
            let json = serde_json::to_string(notif)?;
            writeln!(f, "{}", json)?;
            Ok(())
        }
    }
}

fn truncate(s: &str) -> String {
    if s.len() > 16 {
        format!("{}...{}", &s[..8], &s[s.len() - 6..])
    } else {
        s.to_string()
    }
}

/// Default path for notification center data.
pub fn default_notifications_path() -> std::path::PathBuf {
    crate::config::default_data_dir().join("notifications.json")
}

// ──────────────────────────── Tests ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_center() -> NotificationCenter {
        NotificationCenter::new()
    }

    #[test]
    fn test_notify_basic() {
        let mut center = make_center();
        center.notify(Priority::Medium, EventCategory::System, "Test", "Hello", None);
        assert_eq!(center.len(), 1);
        assert_eq!(center.unread_count(), 1);
    }

    #[test]
    fn test_notify_assigns_id() {
        let mut center = make_center();
        center.notify(Priority::Low, EventCategory::System, "A", "msg", None);
        center.notify(Priority::Low, EventCategory::System, "B", "msg", None);
        assert_ne!(center.history[0].id, center.history[1].id);
    }

    #[test]
    fn test_energy_warning_critical() {
        let mut center = make_center();
        let n = center.energy_warning("0xobj123", 3.0);
        assert_eq!(n.priority, Priority::Critical);
        assert_eq!(n.category, EventCategory::EnergyDecay);
    }

    #[test]
    fn test_energy_warning_high() {
        let mut center = make_center();
        let n = center.energy_warning("0xobj", 10.0);
        assert_eq!(n.priority, Priority::High);
    }

    #[test]
    fn test_energy_warning_medium() {
        let mut center = make_center();
        let n = center.energy_warning("0xobj", 20.0);
        assert_eq!(n.priority, Priority::Medium);
    }

    #[test]
    fn test_energy_warning_low() {
        let mut center = make_center();
        let n = center.energy_warning("0xobj", 50.0);
        assert_eq!(n.priority, Priority::Low);
    }

    #[test]
    fn test_tx_confirmed() {
        let mut center = make_center();
        let n = center.tx_confirmed("0xhash123", "transfer");
        assert_eq!(n.category, EventCategory::TxConfirmed);
        assert_eq!(n.reference.as_deref(), Some("0xhash123"));
    }

    #[test]
    fn test_tx_failed() {
        let mut center = make_center();
        let n = center.tx_failed("0xhash", "insufficient balance");
        assert_eq!(n.priority, Priority::High);
        assert_eq!(n.category, EventCategory::TxFailed);
    }

    #[test]
    fn test_fee_alert() {
        let mut center = make_center();
        let n = center.fee_alert("cheap_gas", 50);
        assert_eq!(n.category, EventCategory::FeeAlert);
        assert!(n.message.contains("50"));
    }

    #[test]
    fn test_security_event() {
        let mut center = make_center();
        let n = center.security_event("Large transfer", "10000 EVAP sent");
        assert_eq!(n.priority, Priority::High);
        assert_eq!(n.category, EventCategory::Security);
    }

    #[test]
    fn test_session_expiring() {
        let mut center = make_center();
        let n = center.session_expiring("sess_123", "EvapSwap");
        assert_eq!(n.category, EventCategory::SessionExpiry);
    }

    #[test]
    fn test_mark_read() {
        let mut center = make_center();
        center.notify(Priority::Low, EventCategory::System, "T", "m", None);
        let id = center.history[0].id.clone();
        assert_eq!(center.unread_count(), 1);
        center.mark_read(&id);
        assert_eq!(center.unread_count(), 0);
    }

    #[test]
    fn test_mark_all_read() {
        let mut center = make_center();
        for i in 0..5 {
            center.notify(Priority::Low, EventCategory::System, &format!("T{}", i), "m", None);
        }
        assert_eq!(center.unread_count(), 5);
        center.mark_all_read();
        assert_eq!(center.unread_count(), 0);
    }

    #[test]
    fn test_filter_by_category() {
        let mut center = make_center();
        center.tx_confirmed("0x1", "transfer");
        center.tx_failed("0x2", "error");
        center.fee_alert("test", 100);

        assert_eq!(center.filter_by_category(EventCategory::TxConfirmed).len(), 1);
        assert_eq!(center.filter_by_category(EventCategory::TxFailed).len(), 1);
        assert_eq!(center.filter_by_category(EventCategory::FeeAlert).len(), 1);
    }

    #[test]
    fn test_filter_by_priority() {
        let mut center = make_center();
        center.notify(Priority::Low, EventCategory::System, "L", "m", None);
        center.notify(Priority::High, EventCategory::System, "H", "m", None);
        center.notify(Priority::Critical, EventCategory::System, "C", "m", None);

        assert_eq!(center.filter_by_priority(Priority::High).len(), 2);
        assert_eq!(center.filter_by_priority(Priority::Critical).len(), 1);
    }

    #[test]
    fn test_recent() {
        let mut center = make_center();
        for i in 0..10 {
            center.notify(Priority::Low, EventCategory::System, &format!("N{}", i), "m", None);
        }
        let recent = center.recent(3);
        assert_eq!(recent.len(), 3);
        assert!(recent[0].title.contains("9")); // most recent first
    }

    #[test]
    fn test_history_trimmed() {
        let mut center = NotificationCenter::with_config(NotificationConfig {
            max_history: 5,
            ..Default::default()
        });
        for i in 0..10 {
            center.notify(Priority::Low, EventCategory::System, &format!("N{}", i), "m", None);
        }
        assert_eq!(center.len(), 5);
    }

    #[test]
    fn test_clear_history() {
        let mut center = make_center();
        center.notify(Priority::Low, EventCategory::System, "T", "m", None);
        center.clear_history();
        assert!(center.is_empty());
    }

    #[test]
    fn test_disabled_notifications() {
        let mut center = NotificationCenter::with_config(NotificationConfig {
            enabled: false,
            ..Default::default()
        });
        // Should still store in history even if disabled
        center.notify(Priority::Critical, EventCategory::System, "T", "m", None);
        assert_eq!(center.len(), 1);
    }

    #[test]
    fn test_priority_ordering() {
        assert!(Priority::Critical > Priority::High);
        assert!(Priority::High > Priority::Medium);
        assert!(Priority::Medium > Priority::Low);
    }

    #[test]
    fn test_priority_labels() {
        assert_eq!(Priority::Critical.label(), "critical");
        assert_eq!(Priority::High.icon(), "ALERT");
    }

    #[test]
    fn test_event_category_label() {
        assert_eq!(EventCategory::EnergyDecay.label(), "energy_decay");
        assert_eq!(EventCategory::TxConfirmed.label(), "tx_confirmed");
    }

    #[test]
    fn test_notification_serializable() {
        let mut center = make_center();
        center.notify(Priority::High, EventCategory::Security, "Alert", "detail", Some("ref123"));
        let json = serde_json::to_string(&center.history[0]).unwrap();
        assert!(json.contains("\"priority\":\"high\""));
        assert!(json.contains("\"reference\":\"ref123\""));
    }

    #[test]
    fn test_json_roundtrip() {
        let mut center = make_center();
        center.tx_confirmed("0x1", "transfer");
        center.fee_alert("test", 50);

        let json = serde_json::to_string_pretty(&center).unwrap();
        let loaded: NotificationCenter = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.len(), 2);
    }

    #[test]
    fn test_file_save_and_load() {
        let dir = std::env::temp_dir().join("evaporchain_notif_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("notifications.json");

        let mut center = make_center();
        center.notify(Priority::Low, EventCategory::System, "T", "m", None);
        center.save(&path).unwrap();

        let loaded = NotificationCenter::load(&path).unwrap();
        assert_eq!(loaded.len(), 1);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_log_file_channel() {
        let log_path = std::env::temp_dir()
            .join("evaporchain_notif_log_test.jsonl")
            .to_string_lossy()
            .to_string();

        let mut center = NotificationCenter::with_config(NotificationConfig {
            channels: vec![NotificationChannel::LogFile { path: log_path.clone() }],
            ..Default::default()
        });
        center.notify(Priority::High, EventCategory::Security, "Test", "msg", None);

        let content = std::fs::read_to_string(&log_path).unwrap();
        assert!(content.contains("\"priority\":\"high\""));

        let _ = std::fs::remove_file(&log_path);
    }

    #[test]
    fn test_truncate_short() {
        assert_eq!(truncate("abc"), "abc");
    }

    #[test]
    fn test_truncate_long() {
        let long = "0x".to_string() + &"ab".repeat(32);
        let t = truncate(&long);
        assert!(t.contains("..."));
    }
}
