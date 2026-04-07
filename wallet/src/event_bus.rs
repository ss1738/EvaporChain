//! Internal event bus for cross-module communication.
//!
//! Provides a publish/subscribe mechanism where modules can register handlers
//! for specific topics (with wildcard support) and publish events that get
//! dispatched to matching handlers. All state is serialisable for persistence.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

// ──────────────────────────── Types ──────────────────────────────────────

#[derive(Debug, Clone, thiserror::Error)]
pub enum EventBusError {
    #[error("handler not found: {0}")]
    HandlerNotFound(String),
    #[error("event not found: {0}")]
    EventNotFound(String),
    #[error("duplicate handler id: {0}")]
    DuplicateHandler(String),
    #[error("handler disabled: {0}")]
    HandlerDisabled(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("json error: {0}")]
    Json(String),
}

impl From<std::io::Error> for EventBusError {
    fn from(e: std::io::Error) -> Self {
        EventBusError::Io(e.to_string())
    }
}
impl From<serde_json::Error> for EventBusError {
    fn from(e: serde_json::Error) -> Self {
        EventBusError::Json(e.to_string())
    }
}

/// Priority of an event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventPriority {
    Low,
    Normal,
    High,
    Critical,
}

/// Status of an event handler.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandlerStatus {
    Active,
    Disabled,
    Failed,
}

/// A single event published on the bus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusEvent {
    pub id: String,
    pub topic: String,
    pub payload: HashMap<String, String>,
    pub priority: EventPriority,
    pub timestamp: String,
    pub source: String,
    pub processed: bool,
}

/// A registered event handler.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventHandler {
    pub id: String,
    /// Topic filter — supports wildcard segments (e.g. `"tx.*"`).
    pub topic_filter: String,
    pub description: String,
    pub status: HandlerStatus,
    pub created_at: String,
    pub invocation_count: u64,
    pub last_invoked: Option<String>,
    pub error_count: u32,
}

/// Log entry produced when a handler processes an event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventLog {
    pub event_id: String,
    pub handler_id: String,
    pub timestamp: String,
    pub success: bool,
    pub duration_ms: u64,
}

/// Aggregate statistics for the event bus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventBusStats {
    pub total_events: usize,
    pub processed_events: usize,
    pub pending_events: usize,
    pub total_handlers: usize,
    pub active_handlers: usize,
    pub total_invocations: u64,
    pub total_errors: u32,
    pub events_per_topic: HashMap<String, usize>,
}

// ──────────────────────────── EventBus ──────────────────────────────────

/// The main event bus store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventBus {
    pub events: Vec<BusEvent>,
    pub handlers: HashMap<String, EventHandler>,
    pub logs: Vec<EventLog>,
    pub max_events: usize,
}

impl Default for EventBus {
    fn default() -> Self {
        Self {
            events: Vec::new(),
            handlers: HashMap::new(),
            logs: Vec::new(),
            max_events: 10_000,
        }
    }
}

impl EventBus {
    /// Create a new event bus with default capacity (10 000 events).
    pub fn new() -> Self {
        Self::default()
    }

    // ── Handler management ──────────────────────────────────────────────

    /// Register a new handler. Fails if the handler id already exists.
    pub fn register_handler(&mut self, handler: EventHandler) -> Result<(), EventBusError> {
        if self.handlers.contains_key(&handler.id) {
            return Err(EventBusError::DuplicateHandler(handler.id.clone()));
        }
        self.handlers.insert(handler.id.clone(), handler);
        Ok(())
    }

    /// Remove and return a handler by id.
    pub fn unregister_handler(&mut self, id: &str) -> Result<EventHandler, EventBusError> {
        self.handlers
            .remove(id)
            .ok_or_else(|| EventBusError::HandlerNotFound(id.to_string()))
    }

    /// Enable a handler.
    pub fn enable_handler(&mut self, id: &str) -> Result<(), EventBusError> {
        let h = self
            .handlers
            .get_mut(id)
            .ok_or_else(|| EventBusError::HandlerNotFound(id.to_string()))?;
        h.status = HandlerStatus::Active;
        Ok(())
    }

    /// Disable a handler.
    pub fn disable_handler(&mut self, id: &str) -> Result<(), EventBusError> {
        let h = self
            .handlers
            .get_mut(id)
            .ok_or_else(|| EventBusError::HandlerNotFound(id.to_string()))?;
        h.status = HandlerStatus::Disabled;
        Ok(())
    }

    // ── Publishing & processing ─────────────────────────────────────────

    /// Publish an event onto the bus. Prunes oldest events when capacity exceeded.
    pub fn publish(&mut self, event: BusEvent) {
        self.events.push(event);
        while self.events.len() > self.max_events {
            self.events.remove(0);
        }
    }

    /// Process an event: find matching active handlers, record logs, mark processed.
    /// Returns the list of handler ids that were invoked.
    pub fn process_event(&mut self, event_id: &str) -> Result<Vec<String>, EventBusError> {
        let event = self
            .events
            .iter()
            .find(|e| e.id == event_id)
            .ok_or_else(|| EventBusError::EventNotFound(event_id.to_string()))?;
        let topic = event.topic.clone();

        // Collect matching active handler ids.
        let handler_ids: Vec<String> = self
            .handlers
            .values()
            .filter(|h| h.status == HandlerStatus::Active && Self::matches_topic(&h.topic_filter, &topic))
            .map(|h| h.id.clone())
            .collect();

        let now = chrono::Utc::now().to_rfc3339();

        for hid in &handler_ids {
            if let Some(h) = self.handlers.get_mut(hid) {
                h.invocation_count += 1;
                h.last_invoked = Some(now.clone());
            }
            self.logs.push(EventLog {
                event_id: event_id.to_string(),
                handler_id: hid.clone(),
                timestamp: now.clone(),
                success: true,
                duration_ms: 0,
            });
        }

        // Mark event as processed.
        if let Some(ev) = self.events.iter_mut().find(|e| e.id == event_id) {
            ev.processed = true;
        }

        Ok(handler_ids)
    }

    /// Check whether a wildcard `filter` matches a `topic`.
    ///
    /// Each dot-separated segment in the filter is compared to the corresponding
    /// segment in the topic. A `"*"` segment matches any single segment.
    pub fn matches_topic(filter: &str, topic: &str) -> bool {
        let filter_parts: Vec<&str> = filter.split('.').collect();
        let topic_parts: Vec<&str> = topic.split('.').collect();

        if filter_parts.len() != topic_parts.len() {
            return false;
        }

        filter_parts
            .iter()
            .zip(topic_parts.iter())
            .all(|(f, t)| *f == "*" || f == t)
    }

    // ── Queries ─────────────────────────────────────────────────────────

    /// Return events that have not yet been processed.
    pub fn pending_events(&self) -> Vec<&BusEvent> {
        self.events.iter().filter(|e| !e.processed).collect()
    }

    /// Return events for a given topic.
    pub fn events_by_topic(&self, topic: &str) -> Vec<&BusEvent> {
        self.events.iter().filter(|e| e.topic == topic).collect()
    }

    /// Return all log entries for a specific handler.
    pub fn handler_logs(&self, handler_id: &str) -> Vec<&EventLog> {
        self.logs.iter().filter(|l| l.handler_id == handler_id).collect()
    }

    /// Return the last `n` log entries (most recent last).
    pub fn recent_logs(&self, n: usize) -> Vec<&EventLog> {
        let start = self.logs.len().saturating_sub(n);
        self.logs[start..].iter().collect()
    }

    /// Return active handlers whose filter matches `topic`.
    pub fn handlers_for_topic(&self, topic: &str) -> Vec<&EventHandler> {
        self.handlers
            .values()
            .filter(|h| h.status == HandlerStatus::Active && Self::matches_topic(&h.topic_filter, topic))
            .collect()
    }

    /// Compute aggregate statistics.
    pub fn stats(&self) -> EventBusStats {
        let processed_events = self.events.iter().filter(|e| e.processed).count();
        let mut events_per_topic: HashMap<String, usize> = HashMap::new();
        for ev in &self.events {
            *events_per_topic.entry(ev.topic.clone()).or_insert(0) += 1;
        }
        EventBusStats {
            total_events: self.events.len(),
            processed_events,
            pending_events: self.events.len() - processed_events,
            total_handlers: self.handlers.len(),
            active_handlers: self
                .handlers
                .values()
                .filter(|h| h.status == HandlerStatus::Active)
                .count(),
            total_invocations: self.handlers.values().map(|h| h.invocation_count).sum(),
            total_errors: self.handlers.values().map(|h| h.error_count).sum(),
            events_per_topic,
        }
    }

    // ── Persistence ─────────────────────────────────────────────────────

    pub fn load(path: &Path) -> Result<Self, EventBusError> {
        let data = std::fs::read_to_string(path)?;
        let bus: EventBus = serde_json::from_str(&data)?;
        Ok(bus)
    }

    pub fn save(&self, path: &Path) -> Result<(), EventBusError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ──────────────────────────── Tests ─────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::temp_dir;
    use std::process::id;

    fn temp_path(name: &str) -> std::path::PathBuf {
        temp_dir().join(format!("event_bus_test_{}_{}", id(), name))
    }

    fn make_handler(id: &str, filter: &str) -> EventHandler {
        EventHandler {
            id: id.to_string(),
            topic_filter: filter.to_string(),
            description: format!("handler {}", id),
            status: HandlerStatus::Active,
            created_at: chrono::Utc::now().to_rfc3339(),
            invocation_count: 0,
            last_invoked: None,
            error_count: 0,
        }
    }

    fn make_event(id: &str, topic: &str) -> BusEvent {
        BusEvent {
            id: id.to_string(),
            topic: topic.to_string(),
            payload: HashMap::new(),
            priority: EventPriority::Normal,
            timestamp: chrono::Utc::now().to_rfc3339(),
            source: "test".to_string(),
            processed: false,
        }
    }

    #[test]
    fn test_new_defaults() {
        let bus = EventBus::new();
        assert_eq!(bus.max_events, 10_000);
        assert!(bus.events.is_empty());
        assert!(bus.handlers.is_empty());
        assert!(bus.logs.is_empty());
    }

    #[test]
    fn test_register_handler() {
        let mut bus = EventBus::new();
        let h = make_handler("h1", "tx.*");
        assert!(bus.register_handler(h).is_ok());
        assert_eq!(bus.handlers.len(), 1);
    }

    #[test]
    fn test_register_duplicate_handler() {
        let mut bus = EventBus::new();
        bus.register_handler(make_handler("h1", "tx.*")).unwrap();
        let res = bus.register_handler(make_handler("h1", "tx.*"));
        assert!(res.is_err());
    }

    #[test]
    fn test_unregister_handler() {
        let mut bus = EventBus::new();
        bus.register_handler(make_handler("h1", "tx.*")).unwrap();
        let h = bus.unregister_handler("h1").unwrap();
        assert_eq!(h.id, "h1");
        assert!(bus.handlers.is_empty());
    }

    #[test]
    fn test_unregister_missing_handler() {
        let mut bus = EventBus::new();
        assert!(bus.unregister_handler("nope").is_err());
    }

    #[test]
    fn test_enable_disable_handler() {
        let mut bus = EventBus::new();
        bus.register_handler(make_handler("h1", "tx.*")).unwrap();
        bus.disable_handler("h1").unwrap();
        assert_eq!(bus.handlers["h1"].status, HandlerStatus::Disabled);
        bus.enable_handler("h1").unwrap();
        assert_eq!(bus.handlers["h1"].status, HandlerStatus::Active);
    }

    #[test]
    fn test_enable_missing_handler() {
        let mut bus = EventBus::new();
        assert!(bus.enable_handler("nope").is_err());
    }

    #[test]
    fn test_disable_missing_handler() {
        let mut bus = EventBus::new();
        assert!(bus.disable_handler("nope").is_err());
    }

    #[test]
    fn test_publish_event() {
        let mut bus = EventBus::new();
        bus.publish(make_event("e1", "tx.sent"));
        assert_eq!(bus.events.len(), 1);
        assert_eq!(bus.events[0].id, "e1");
    }

    #[test]
    fn test_publish_prunes_old_events() {
        let mut bus = EventBus::new();
        bus.max_events = 3;
        for i in 0..5 {
            bus.publish(make_event(&format!("e{}", i), "tx.sent"));
        }
        assert_eq!(bus.events.len(), 3);
        assert_eq!(bus.events[0].id, "e2");
    }

    #[test]
    fn test_matches_topic_exact() {
        assert!(EventBus::matches_topic("tx.sent", "tx.sent"));
        assert!(!EventBus::matches_topic("tx.sent", "tx.received"));
    }

    #[test]
    fn test_matches_topic_wildcard() {
        assert!(EventBus::matches_topic("tx.*", "tx.sent"));
        assert!(EventBus::matches_topic("tx.*", "tx.received"));
        assert!(!EventBus::matches_topic("tx.*", "block.new"));
    }

    #[test]
    fn test_matches_topic_length_mismatch() {
        assert!(!EventBus::matches_topic("tx.*", "tx.sent.confirmed"));
        assert!(!EventBus::matches_topic("tx.*.*", "tx.sent"));
    }

    #[test]
    fn test_process_event() {
        let mut bus = EventBus::new();
        bus.register_handler(make_handler("h1", "tx.*")).unwrap();
        bus.register_handler(make_handler("h2", "block.*")).unwrap();
        bus.publish(make_event("e1", "tx.sent"));
        let ids = bus.process_event("e1").unwrap();
        assert_eq!(ids, vec!["h1".to_string()]);
        assert!(bus.events[0].processed);
        assert_eq!(bus.handlers["h1"].invocation_count, 1);
        assert!(bus.handlers["h1"].last_invoked.is_some());
        assert_eq!(bus.logs.len(), 1);
    }

    #[test]
    fn test_process_event_not_found() {
        let mut bus = EventBus::new();
        assert!(bus.process_event("nope").is_err());
    }

    #[test]
    fn test_process_event_disabled_handler_skipped() {
        let mut bus = EventBus::new();
        bus.register_handler(make_handler("h1", "tx.*")).unwrap();
        bus.disable_handler("h1").unwrap();
        bus.publish(make_event("e1", "tx.sent"));
        let ids = bus.process_event("e1").unwrap();
        assert!(ids.is_empty());
    }

    #[test]
    fn test_pending_events() {
        let mut bus = EventBus::new();
        bus.publish(make_event("e1", "tx.sent"));
        bus.publish(make_event("e2", "tx.sent"));
        bus.events[0].processed = true;
        let pending = bus.pending_events();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "e2");
    }

    #[test]
    fn test_events_by_topic() {
        let mut bus = EventBus::new();
        bus.publish(make_event("e1", "tx.sent"));
        bus.publish(make_event("e2", "block.new"));
        bus.publish(make_event("e3", "tx.sent"));
        let evts = bus.events_by_topic("tx.sent");
        assert_eq!(evts.len(), 2);
    }

    #[test]
    fn test_handlers_for_topic() {
        let mut bus = EventBus::new();
        bus.register_handler(make_handler("h1", "tx.*")).unwrap();
        bus.register_handler(make_handler("h2", "block.*")).unwrap();
        let hs = bus.handlers_for_topic("tx.sent");
        assert_eq!(hs.len(), 1);
        assert_eq!(hs[0].id, "h1");
    }

    #[test]
    fn test_recent_logs() {
        let mut bus = EventBus::new();
        for i in 0..5 {
            bus.logs.push(EventLog {
                event_id: format!("e{}", i),
                handler_id: "h1".to_string(),
                timestamp: chrono::Utc::now().to_rfc3339(),
                success: true,
                duration_ms: 0,
            });
        }
        let recent = bus.recent_logs(3);
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].event_id, "e2");
    }

    #[test]
    fn test_handler_logs() {
        let mut bus = EventBus::new();
        bus.logs.push(EventLog {
            event_id: "e1".to_string(),
            handler_id: "h1".to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            success: true,
            duration_ms: 0,
        });
        bus.logs.push(EventLog {
            event_id: "e2".to_string(),
            handler_id: "h2".to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            success: true,
            duration_ms: 0,
        });
        let logs = bus.handler_logs("h1");
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].event_id, "e1");
    }

    #[test]
    fn test_stats() {
        let mut bus = EventBus::new();
        bus.register_handler(make_handler("h1", "tx.*")).unwrap();
        bus.publish(make_event("e1", "tx.sent"));
        bus.publish(make_event("e2", "tx.received"));
        bus.process_event("e1").unwrap();

        let s = bus.stats();
        assert_eq!(s.total_events, 2);
        assert_eq!(s.processed_events, 1);
        assert_eq!(s.pending_events, 1);
        assert_eq!(s.total_handlers, 1);
        assert_eq!(s.active_handlers, 1);
        assert_eq!(s.total_invocations, 1);
        assert_eq!(s.total_errors, 0);
        assert_eq!(s.events_per_topic["tx.sent"], 1);
        assert_eq!(s.events_per_topic["tx.received"], 1);
    }

    #[test]
    fn test_save_and_load() {
        let path = temp_path("bus.json");
        let mut bus = EventBus::new();
        bus.register_handler(make_handler("h1", "tx.*")).unwrap();
        bus.publish(make_event("e1", "tx.sent"));
        bus.save(&path).unwrap();

        let loaded = EventBus::load(&path).unwrap();
        assert_eq!(loaded.events.len(), 1);
        assert_eq!(loaded.handlers.len(), 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_or_default_missing_file() {
        let path = temp_path("nonexistent.json");
        let bus = EventBus::load_or_default(&path);
        assert!(bus.events.is_empty());
        assert_eq!(bus.max_events, 10_000);
    }
}
