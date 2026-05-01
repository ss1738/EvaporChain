use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum TelemetryError {
    #[error("telemetry not enabled: {0}")]
    NotEnabled(String),
    #[error("event not found: {0}")]
    EventNotFound(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Parse(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum TelemetryLevel {
    #[default]
    Off,
    Basic,
    Detailed,
    Full,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EventCategory2 {
    Command,
    Transaction,
    Error,
    Performance,
    Feature,
    Custom(String),
}

// ---------------------------------------------------------------------------
// Structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryEvent {
    pub id: String,
    pub category: EventCategory2,
    pub name: String,
    pub properties: HashMap<String, String>,
    pub timestamp: String,
    pub session_id: String,
    pub anonymized: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryConfig2 {
    pub level: TelemetryLevel,
    pub opted_in: bool,
    pub anonymize: bool,
    pub session_id: String,
    pub max_events: usize,
    pub flush_interval_seconds: u64,
}

impl Default for TelemetryConfig2 {
    fn default() -> Self {
        Self {
            level: TelemetryLevel::Off,
            opted_in: false,
            anonymize: true,
            session_id: String::new(),
            max_events: 10000,
            flush_interval_seconds: 300,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsagePattern {
    pub command: String,
    pub count: u64,
    pub last_used: String,
    pub avg_duration_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryStats2 {
    pub enabled: bool,
    pub level: TelemetryLevel,
    pub total_events: usize,
    pub events_by_category: HashMap<String, usize>,
    pub unique_commands: usize,
    pub session_count: u64,
    pub anonymized_events: usize,
}

// ---------------------------------------------------------------------------
// TelemetryManager
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TelemetryManager {
    pub config: TelemetryConfig2,
    pub events: Vec<TelemetryEvent>,
    pub patterns: HashMap<String, UsagePattern>,
    pub session_count: u64,
}

impl TelemetryManager {
    // -- construction -------------------------------------------------------

    pub fn new() -> Self {
        Self::default()
    }

    // -- opt-in / opt-out ---------------------------------------------------

    pub fn opt_in(&mut self, level: TelemetryLevel) {
        self.config.opted_in = true;
        self.config.level = level;
        self.config.session_id = generate_id();
    }

    pub fn opt_out(&mut self) {
        self.config.opted_in = false;
        self.config.level = TelemetryLevel::Off;
        self.events.clear();
    }

    pub fn is_enabled(&self) -> bool {
        self.config.opted_in
    }

    // -- recording ----------------------------------------------------------

    pub fn record_event(
        &mut self,
        category: EventCategory2,
        name: &str,
        mut properties: HashMap<String, String>,
    ) -> Result<String, TelemetryError> {
        if !self.config.opted_in {
            return Err(TelemetryError::NotEnabled(
                "telemetry is not opted in".into(),
            ));
        }

        if self.config.anonymize {
            properties.retain(|k, _| {
                let lower = k.to_lowercase();
                !lower.contains("address") && !lower.contains("key") && !lower.contains("hash")
            });
        }

        let id = generate_id();
        let event = TelemetryEvent {
            id: id.clone(),
            category,
            name: name.to_string(),
            properties,
            timestamp: Utc::now().to_rfc3339(),
            session_id: self.config.session_id.clone(),
            anonymized: self.config.anonymize,
        };

        self.events.push(event);

        // Trim oldest events when we exceed the cap.
        if self.events.len() > self.config.max_events {
            let overflow = self.events.len() - self.config.max_events;
            self.events.drain(..overflow);
        }

        Ok(id)
    }

    pub fn record_command(
        &mut self,
        command: &str,
        duration_ms: f64,
    ) -> Result<(), TelemetryError> {
        if !self.config.opted_in {
            return Err(TelemetryError::NotEnabled(
                "telemetry is not opted in".into(),
            ));
        }

        let now = Utc::now().to_rfc3339();
        let pattern = self
            .patterns
            .entry(command.to_string())
            .or_insert_with(|| UsagePattern {
                command: command.to_string(),
                count: 0,
                last_used: now.clone(),
                avg_duration_ms: 0.0,
            });

        let total_duration = pattern.avg_duration_ms * pattern.count as f64;
        pattern.count += 1;
        pattern.avg_duration_ms = (total_duration + duration_ms) / pattern.count as f64;
        pattern.last_used = now;

        Ok(())
    }

    // -- queries ------------------------------------------------------------

    pub fn get_event(&self, id: &str) -> Option<&TelemetryEvent> {
        self.events.iter().find(|e| e.id == id)
    }

    pub fn events_by_category(&self, cat: &EventCategory2) -> Vec<&TelemetryEvent> {
        self.events.iter().filter(|e| &e.category == cat).collect()
    }

    pub fn recent_events(&self, n: usize) -> Vec<&TelemetryEvent> {
        self.events.iter().rev().take(n).collect()
    }

    pub fn top_commands(&self, n: usize) -> Vec<&UsagePattern> {
        let mut sorted: Vec<&UsagePattern> = self.patterns.values().collect();
        sorted.sort_by_key(|a| std::cmp::Reverse(a.count));
        sorted.truncate(n);
        sorted
    }

    // -- session ------------------------------------------------------------

    pub fn new_session(&mut self) {
        self.config.session_id = generate_id();
        self.session_count += 1;
    }

    // -- flush --------------------------------------------------------------

    pub fn flush(&mut self) -> Vec<TelemetryEvent> {
        std::mem::take(&mut self.events)
    }

    // -- anonymization ------------------------------------------------------

    pub fn anonymize_event(event: &mut TelemetryEvent) {
        event.properties.retain(|k, _| {
            let lower = k.to_lowercase();
            !lower.contains("address") && !lower.contains("key") && !lower.contains("hash")
        });
        event.anonymized = true;
    }

    // -- stats --------------------------------------------------------------

    pub fn stats(&self) -> TelemetryStats2 {
        let mut events_by_category: HashMap<String, usize> = HashMap::new();
        let mut anonymized_events = 0usize;

        for event in &self.events {
            let cat_name = match &event.category {
                EventCategory2::Command => "Command".to_string(),
                EventCategory2::Transaction => "Transaction".to_string(),
                EventCategory2::Error => "Error".to_string(),
                EventCategory2::Performance => "Performance".to_string(),
                EventCategory2::Feature => "Feature".to_string(),
                EventCategory2::Custom(s) => format!("Custom({})", s),
            };
            *events_by_category.entry(cat_name).or_insert(0) += 1;
            if event.anonymized {
                anonymized_events += 1;
            }
        }

        TelemetryStats2 {
            enabled: self.config.opted_in,
            level: self.config.level.clone(),
            total_events: self.events.len(),
            events_by_category,
            unique_commands: self.patterns.len(),
            session_count: self.session_count,
            anonymized_events,
        }
    }

    // -- persistence --------------------------------------------------------

    pub fn save(&self, path: &Path) -> Result<(), TelemetryError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, TelemetryError> {
        let data = std::fs::read_to_string(path)?;
        let manager: Self = serde_json::from_str(&data)?;
        Ok(manager)
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn generate_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:x}-{:x}", nanos, rand_simple())
}

/// Cheap pseudo-random u64 (no external crate needed).
fn rand_simple() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    // xorshift-style mix
    let mut x = seed;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn test_file_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "evaporchain_telemetry_test_{}_{}",
            std::process::id(),
            name
        ))
    }

    // 1
    #[test]
    fn test_default_config_is_off() {
        let cfg = TelemetryConfig2::default();
        assert_eq!(cfg.level, TelemetryLevel::Off);
        assert!(!cfg.opted_in);
        assert!(cfg.anonymize);
        assert!(cfg.session_id.is_empty());
        assert_eq!(cfg.max_events, 10000);
        assert_eq!(cfg.flush_interval_seconds, 300);
    }

    // 2
    #[test]
    fn test_new_manager_not_enabled() {
        let mgr = TelemetryManager::new();
        assert!(!mgr.is_enabled());
    }

    // 3
    #[test]
    fn test_opt_in() {
        let mut mgr = TelemetryManager::new();
        mgr.opt_in(TelemetryLevel::Basic);
        assert!(mgr.is_enabled());
        assert_eq!(mgr.config.level, TelemetryLevel::Basic);
        assert!(!mgr.config.session_id.is_empty());
    }

    // 4
    #[test]
    fn test_opt_out() {
        let mut mgr = TelemetryManager::new();
        mgr.opt_in(TelemetryLevel::Full);
        let _ = mgr.record_event(EventCategory2::Command, "test", HashMap::new());
        assert!(!mgr.events.is_empty());
        mgr.opt_out();
        assert!(!mgr.is_enabled());
        assert_eq!(mgr.config.level, TelemetryLevel::Off);
        assert!(mgr.events.is_empty());
    }

    // 5
    #[test]
    fn test_record_event_when_disabled() {
        let mut mgr = TelemetryManager::new();
        let result = mgr.record_event(EventCategory2::Command, "test", HashMap::new());
        assert!(result.is_err());
        match result {
            Err(TelemetryError::NotEnabled(_)) => {}
            _ => panic!("expected NotEnabled"),
        }
    }

    // 6
    #[test]
    fn test_record_event_when_enabled() {
        let mut mgr = TelemetryManager::new();
        mgr.opt_in(TelemetryLevel::Detailed);
        let id = mgr
            .record_event(EventCategory2::Transaction, "send", HashMap::new())
            .unwrap();
        assert!(!id.is_empty());
        assert_eq!(mgr.events.len(), 1);
        assert_eq!(mgr.events[0].name, "send");
    }

    // 7
    #[test]
    fn test_anonymization_strips_sensitive_keys() {
        let mut mgr = TelemetryManager::new();
        mgr.opt_in(TelemetryLevel::Full);
        let mut props = HashMap::new();
        props.insert("wallet_address".into(), "0xABC".into());
        props.insert("private_key".into(), "secret".into());
        props.insert("tx_hash".into(), "0x123".into());
        props.insert("amount".into(), "100".into());
        let id = mgr
            .record_event(EventCategory2::Transaction, "send", props)
            .unwrap();
        let evt = mgr.get_event(&id).unwrap();
        assert!(!evt.properties.contains_key("wallet_address"));
        assert!(!evt.properties.contains_key("private_key"));
        assert!(!evt.properties.contains_key("tx_hash"));
        assert!(evt.properties.contains_key("amount"));
    }

    // 8
    #[test]
    fn test_anonymization_disabled() {
        let mut mgr = TelemetryManager::new();
        mgr.opt_in(TelemetryLevel::Full);
        mgr.config.anonymize = false;
        let mut props = HashMap::new();
        props.insert("wallet_address".into(), "0xABC".into());
        let id = mgr
            .record_event(EventCategory2::Transaction, "send", props)
            .unwrap();
        let evt = mgr.get_event(&id).unwrap();
        assert!(evt.properties.contains_key("wallet_address"));
    }

    // 9
    #[test]
    fn test_record_command_updates_pattern() {
        let mut mgr = TelemetryManager::new();
        mgr.opt_in(TelemetryLevel::Basic);
        mgr.record_command("send", 100.0).unwrap();
        mgr.record_command("send", 200.0).unwrap();
        let pattern = mgr.patterns.get("send").unwrap();
        assert_eq!(pattern.count, 2);
        assert!((pattern.avg_duration_ms - 150.0).abs() < f64::EPSILON);
    }

    // 10
    #[test]
    fn test_record_command_when_disabled() {
        let mut mgr = TelemetryManager::new();
        let result = mgr.record_command("send", 50.0);
        assert!(result.is_err());
    }

    // 11
    #[test]
    fn test_events_by_category() {
        let mut mgr = TelemetryManager::new();
        mgr.opt_in(TelemetryLevel::Full);
        mgr.record_event(EventCategory2::Command, "a", HashMap::new())
            .unwrap();
        mgr.record_event(EventCategory2::Transaction, "b", HashMap::new())
            .unwrap();
        mgr.record_event(EventCategory2::Command, "c", HashMap::new())
            .unwrap();
        let cmds = mgr.events_by_category(&EventCategory2::Command);
        assert_eq!(cmds.len(), 2);
        let txs = mgr.events_by_category(&EventCategory2::Transaction);
        assert_eq!(txs.len(), 1);
    }

    // 12
    #[test]
    fn test_recent_events() {
        let mut mgr = TelemetryManager::new();
        mgr.opt_in(TelemetryLevel::Full);
        for i in 0..5 {
            mgr.record_event(
                EventCategory2::Command,
                &format!("cmd_{}", i),
                HashMap::new(),
            )
            .unwrap();
        }
        let recent = mgr.recent_events(3);
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].name, "cmd_4");
        assert_eq!(recent[2].name, "cmd_2");
    }

    // 13
    #[test]
    fn test_top_commands() {
        let mut mgr = TelemetryManager::new();
        mgr.opt_in(TelemetryLevel::Basic);
        mgr.record_command("send", 10.0).unwrap();
        mgr.record_command("send", 20.0).unwrap();
        mgr.record_command("send", 30.0).unwrap();
        mgr.record_command("balance", 5.0).unwrap();
        mgr.record_command("stake", 15.0).unwrap();
        mgr.record_command("stake", 25.0).unwrap();
        let top = mgr.top_commands(2);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].command, "send");
        assert_eq!(top[1].command, "stake");
    }

    // 14
    #[test]
    fn test_new_session() {
        let mut mgr = TelemetryManager::new();
        mgr.opt_in(TelemetryLevel::Basic);
        let first_session = mgr.config.session_id.clone();
        // sleep a tiny bit so the timestamp-based id differs
        std::thread::sleep(std::time::Duration::from_millis(2));
        mgr.new_session();
        assert_ne!(mgr.config.session_id, first_session);
        assert_eq!(mgr.session_count, 1);
        mgr.new_session();
        assert_eq!(mgr.session_count, 2);
    }

    // 15
    #[test]
    fn test_flush() {
        let mut mgr = TelemetryManager::new();
        mgr.opt_in(TelemetryLevel::Full);
        mgr.record_event(EventCategory2::Command, "a", HashMap::new())
            .unwrap();
        mgr.record_event(EventCategory2::Error, "b", HashMap::new())
            .unwrap();
        let flushed = mgr.flush();
        assert_eq!(flushed.len(), 2);
        assert!(mgr.events.is_empty());
    }

    // 16
    #[test]
    fn test_max_events_trim() {
        let mut mgr = TelemetryManager::new();
        mgr.opt_in(TelemetryLevel::Full);
        mgr.config.max_events = 5;
        for i in 0..8 {
            mgr.record_event(
                EventCategory2::Command,
                &format!("evt_{}", i),
                HashMap::new(),
            )
            .unwrap();
        }
        assert_eq!(mgr.events.len(), 5);
        // oldest should have been trimmed
        assert_eq!(mgr.events[0].name, "evt_3");
    }

    // 17
    #[test]
    fn test_stats() {
        let mut mgr = TelemetryManager::new();
        mgr.opt_in(TelemetryLevel::Detailed);
        mgr.record_event(EventCategory2::Command, "a", HashMap::new())
            .unwrap();
        mgr.record_event(EventCategory2::Command, "b", HashMap::new())
            .unwrap();
        mgr.record_event(EventCategory2::Error, "c", HashMap::new())
            .unwrap();
        mgr.record_command("send", 10.0).unwrap();
        mgr.record_command("balance", 20.0).unwrap();
        let stats = mgr.stats();
        assert!(stats.enabled);
        assert_eq!(stats.level, TelemetryLevel::Detailed);
        assert_eq!(stats.total_events, 3);
        assert_eq!(stats.events_by_category.get("Command"), Some(&2));
        assert_eq!(stats.events_by_category.get("Error"), Some(&1));
        assert_eq!(stats.unique_commands, 2);
    }

    // 18
    #[test]
    fn test_save_and_load() {
        let path = test_file_path("save_load.json");
        let mut mgr = TelemetryManager::new();
        mgr.opt_in(TelemetryLevel::Full);
        mgr.record_event(EventCategory2::Feature, "x", HashMap::new())
            .unwrap();
        mgr.record_command("send", 42.0).unwrap();
        mgr.save(&path).unwrap();

        let loaded = TelemetryManager::load(&path).unwrap();
        assert_eq!(loaded.events.len(), 1);
        assert_eq!(loaded.events[0].name, "x");
        assert_eq!(loaded.patterns.get("send").unwrap().count, 1);
        let _ = std::fs::remove_file(&path);
    }

    // 19
    #[test]
    fn test_load_or_default_missing_file() {
        let path = test_file_path("nonexistent.json");
        let _ = std::fs::remove_file(&path); // ensure absent
        let mgr = TelemetryManager::load_or_default(&path);
        assert!(!mgr.is_enabled());
        assert_eq!(mgr.events.len(), 0);
    }

    // 20
    #[test]
    fn test_is_enabled_reflects_opt_state() {
        let mut mgr = TelemetryManager::new();
        assert!(!mgr.is_enabled());
        mgr.opt_in(TelemetryLevel::Basic);
        assert!(mgr.is_enabled());
        mgr.opt_out();
        assert!(!mgr.is_enabled());
    }

    // 21
    #[test]
    fn test_anonymize_event_static() {
        let mut event = TelemetryEvent {
            id: "test-id".into(),
            category: EventCategory2::Transaction,
            name: "transfer".into(),
            properties: {
                let mut m = HashMap::new();
                m.insert("from_address".into(), "0xABC".into());
                m.insert("api_key".into(), "secret".into());
                m.insert("block_hash".into(), "0x999".into());
                m.insert("value".into(), "500".into());
                m
            },
            timestamp: Utc::now().to_rfc3339(),
            session_id: "sess".into(),
            anonymized: false,
        };
        TelemetryManager::anonymize_event(&mut event);
        assert!(event.anonymized);
        assert!(!event.properties.contains_key("from_address"));
        assert!(!event.properties.contains_key("api_key"));
        assert!(!event.properties.contains_key("block_hash"));
        assert!(event.properties.contains_key("value"));
    }
}
