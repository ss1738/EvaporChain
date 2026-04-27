//! WebSocket subscription manager for real-time blockchain events.
//!
//! Manages subscriptions to various blockchain event streams over WebSocket
//! connections, with support for filtering, pause/resume, automatic
//! disconnection on repeated errors, and event history tracking.
//!
//! Features:
//! - Subscribe to multiple event types (blocks, transactions, tokens, etc.)
//! - Filter events by field-level predicates
//! - Pause, resume, and reconnect subscriptions
//! - Automatic disconnection after excessive errors
//! - Bounded event history with configurable max size
//! - Aggregate statistics

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum WsSubscriberError {
    #[error("subscription already exists: {0}")]
    AlreadyExists(String),
    #[error("subscription not found: {0}")]
    NotFound(String),
    #[error("subscription not active: {0}")]
    NotActive(String),
    #[error("subscription not paused: {0}")]
    NotPaused(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Parse(#[from] serde_json::Error),
}

// ──────────────────────────── Enums ──────────────────────────────────────

/// Status of a WebSocket subscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubStatus {
    Active,
    Paused,
    Disconnected,
    Error,
}

/// Type of blockchain event to subscribe to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventType {
    NewBlock,
    PendingTx,
    ConfirmedTx,
    TokenTransfer,
    ContractEvent,
    EnergyDecay,
    PriceUpdate,
    Custom(String),
}

/// Comparison operator for event filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilterOp {
    Eq,
    Gt,
    Lt,
    Gte,
    Lte,
    Contains,
}

// ──────────────────────────── Structs ──────────────────────────────────

/// A single WebSocket subscription to a blockchain event stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub id: String,
    pub event_type: EventType,
    pub status: SubStatus,
    pub filters: Vec<EventFilter>,
    pub created_at: String,
    pub last_event: Option<String>,
    pub event_count: u64,
    pub error_count: u32,
    pub endpoint: String,
}

/// A field-level filter applied to incoming events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventFilter {
    pub field: String,
    pub op: FilterOp,
    pub value: String,
}

/// An event received from a WebSocket subscription.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceivedEvent {
    pub subscription_id: String,
    pub event_type: EventType,
    pub timestamp: String,
    pub payload: HashMap<String, String>,
    pub block_number: Option<u64>,
}

/// Aggregate statistics for the WebSocket subscriber.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsStats {
    pub total_subscriptions: usize,
    pub active: usize,
    pub paused: usize,
    pub disconnected: usize,
    pub total_events: u64,
    pub total_errors: u32,
    pub events_per_minute: f64,
}

// ──────────────────────────── WsSubscriber ──────────────────────────

/// Main store for WebSocket subscriptions and received events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsSubscriber {
    pub subscriptions: HashMap<String, Subscription>,
    pub events: Vec<ReceivedEvent>,
    pub max_events: usize,
}

impl Default for WsSubscriber {
    fn default() -> Self {
        Self::new()
    }
}

impl WsSubscriber {
    /// Create a new subscriber with default max_events of 5000.
    pub fn new() -> Self {
        Self {
            subscriptions: HashMap::new(),
            events: Vec::new(),
            max_events: 5000,
        }
    }

    /// Add a new subscription. Fails if the id already exists.
    pub fn subscribe(&mut self, sub: Subscription) -> Result<(), WsSubscriberError> {
        if self.subscriptions.contains_key(&sub.id) {
            return Err(WsSubscriberError::AlreadyExists(sub.id));
        }
        self.subscriptions.insert(sub.id.clone(), sub);
        Ok(())
    }

    /// Remove and return a subscription by id.
    pub fn unsubscribe(&mut self, id: &str) -> Result<Subscription, WsSubscriberError> {
        self.subscriptions
            .remove(id)
            .ok_or_else(|| WsSubscriberError::NotFound(id.to_string()))
    }

    /// Pause an active subscription.
    pub fn pause(&mut self, id: &str) -> Result<(), WsSubscriberError> {
        let sub = self
            .subscriptions
            .get_mut(id)
            .ok_or_else(|| WsSubscriberError::NotFound(id.to_string()))?;
        if sub.status != SubStatus::Active {
            return Err(WsSubscriberError::NotActive(id.to_string()));
        }
        sub.status = SubStatus::Paused;
        Ok(())
    }

    /// Resume a paused subscription.
    pub fn resume(&mut self, id: &str) -> Result<(), WsSubscriberError> {
        let sub = self
            .subscriptions
            .get_mut(id)
            .ok_or_else(|| WsSubscriberError::NotFound(id.to_string()))?;
        if sub.status != SubStatus::Paused {
            return Err(WsSubscriberError::NotPaused(id.to_string()));
        }
        sub.status = SubStatus::Active;
        Ok(())
    }

    /// Record a received event. Increments the subscription's event_count,
    /// updates last_event timestamp, and prunes old events if over max_events.
    pub fn record_event(&mut self, event: ReceivedEvent) -> Result<(), WsSubscriberError> {
        let sub = self
            .subscriptions
            .get_mut(&event.subscription_id)
            .ok_or_else(|| WsSubscriberError::NotFound(event.subscription_id.clone()))?;
        sub.event_count += 1;
        sub.last_event = Some(event.timestamp.clone());
        self.events.push(event);
        if self.events.len() > self.max_events {
            let excess = self.events.len() - self.max_events;
            self.events.drain(..excess);
        }
        Ok(())
    }

    /// Record an error for a subscription. Increments error_count and sets
    /// status to Disconnected if errors exceed 10.
    pub fn record_error(&mut self, sub_id: &str, _error: &str) -> Result<(), WsSubscriberError> {
        let sub = self
            .subscriptions
            .get_mut(sub_id)
            .ok_or_else(|| WsSubscriberError::NotFound(sub_id.to_string()))?;
        sub.error_count += 1;
        if sub.error_count > 10 {
            sub.status = SubStatus::Disconnected;
        }
        Ok(())
    }

    /// Check whether a received event matches all the given filters.
    pub fn matches_filters(event: &ReceivedEvent, filters: &[EventFilter]) -> bool {
        filters.iter().all(|f| {
            let Some(val) = event.payload.get(&f.field) else {
                return false;
            };
            match f.op {
                FilterOp::Eq => val == &f.value,
                FilterOp::Contains => val.contains(&f.value),
                FilterOp::Gt => {
                    val.parse::<f64>()
                        .ok()
                        .zip(f.value.parse::<f64>().ok())
                        .map_or(false, |(v, t)| v > t)
                }
                FilterOp::Lt => {
                    val.parse::<f64>()
                        .ok()
                        .zip(f.value.parse::<f64>().ok())
                        .map_or(false, |(v, t)| v < t)
                }
                FilterOp::Gte => {
                    val.parse::<f64>()
                        .ok()
                        .zip(f.value.parse::<f64>().ok())
                        .map_or(false, |(v, t)| v >= t)
                }
                FilterOp::Lte => {
                    val.parse::<f64>()
                        .ok()
                        .zip(f.value.parse::<f64>().ok())
                        .map_or(false, |(v, t)| v <= t)
                }
            }
        })
    }

    /// Return all events belonging to a given subscription.
    pub fn events_for_subscription(&self, id: &str) -> Vec<&ReceivedEvent> {
        self.events
            .iter()
            .filter(|e| e.subscription_id == id)
            .collect()
    }

    /// Return the most recent `n` events across all subscriptions.
    pub fn recent_events(&self, n: usize) -> Vec<&ReceivedEvent> {
        let start = self.events.len().saturating_sub(n);
        self.events[start..].iter().collect()
    }

    /// Return all subscriptions with Active status.
    pub fn active_subscriptions(&self) -> Vec<&Subscription> {
        self.subscriptions
            .values()
            .filter(|s| s.status == SubStatus::Active)
            .collect()
    }

    /// Return all subscriptions matching a given event type.
    pub fn subscriptions_by_type(&self, event_type: &EventType) -> Vec<&Subscription> {
        self.subscriptions
            .values()
            .filter(|s| &s.event_type == event_type)
            .collect()
    }

    /// Reconnect a disconnected subscription: reset error_count and set Active.
    pub fn reconnect(&mut self, id: &str) -> Result<(), WsSubscriberError> {
        let sub = self
            .subscriptions
            .get_mut(id)
            .ok_or_else(|| WsSubscriberError::NotFound(id.to_string()))?;
        sub.error_count = 0;
        sub.status = SubStatus::Active;
        Ok(())
    }

    /// Compute aggregate statistics for the subscriber.
    pub fn stats(&self) -> WsStats {
        let total_subscriptions = self.subscriptions.len();
        let active = self
            .subscriptions
            .values()
            .filter(|s| s.status == SubStatus::Active)
            .count();
        let paused = self
            .subscriptions
            .values()
            .filter(|s| s.status == SubStatus::Paused)
            .count();
        let disconnected = self
            .subscriptions
            .values()
            .filter(|s| s.status == SubStatus::Disconnected)
            .count();
        let total_events: u64 = self.subscriptions.values().map(|s| s.event_count).sum();
        let total_errors: u32 = self.subscriptions.values().map(|s| s.error_count).sum();

        // Estimate events per minute from the last 100 events.
        let epm = if self.events.len() >= 2 {
            let window = &self.events[self.events.len().saturating_sub(100)..];
            if let (Some(first), Some(last)) = (window.first(), window.last()) {
                let t0 = chrono::DateTime::parse_from_rfc3339(&first.timestamp).ok();
                let t1 = chrono::DateTime::parse_from_rfc3339(&last.timestamp).ok();
                if let (Some(t0), Some(t1)) = (t0, t1) {
                    let secs = (t1 - t0).num_seconds().max(1) as f64;
                    (window.len() as f64 / secs) * 60.0
                } else {
                    0.0
                }
            } else {
                0.0
            }
        } else {
            0.0
        };

        WsStats {
            total_subscriptions,
            active,
            paused,
            disconnected,
            total_events,
            total_errors,
            events_per_minute: epm,
        }
    }

    /// Save the subscriber state to a JSON file.
    pub fn save(&self, path: &Path) -> Result<(), WsSubscriberError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Load the subscriber state from a JSON file.
    pub fn load(path: &Path) -> Result<Self, WsSubscriberError> {
        let data = std::fs::read_to_string(path)?;
        let subscriber: Self = serde_json::from_str(&data)?;
        Ok(subscriber)
    }

    /// Load from file or return a default instance if the file cannot be read.
    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ──────────────────────────── Helpers ──────────────────────────────────

/// Create a subscription with sensible defaults.
#[cfg(test)]
fn make_sub(id: &str, event_type: EventType, endpoint: &str) -> Subscription {
    Subscription {
        id: id.to_string(),
        event_type,
        status: SubStatus::Active,
        filters: Vec::new(),
        created_at: chrono::Utc::now().to_rfc3339(),
        last_event: None,
        event_count: 0,
        error_count: 0,
        endpoint: endpoint.to_string(),
    }
}

/// Create a received event with minimal fields.
#[cfg(test)]
fn make_event(sub_id: &str, event_type: EventType) -> ReceivedEvent {
    ReceivedEvent {
        subscription_id: sub_id.to_string(),
        event_type,
        timestamp: chrono::Utc::now().to_rfc3339(),
        payload: HashMap::new(),
        block_number: None,
    }
}

// ──────────────────────────── Tests ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("ws_subscriber_test_{}_{}", name, std::process::id()))
    }

    #[test]
    fn test_new_defaults() {
        let ws = WsSubscriber::new();
        assert_eq!(ws.max_events, 5000);
        assert!(ws.subscriptions.is_empty());
        assert!(ws.events.is_empty());
    }

    #[test]
    fn test_subscribe_ok() {
        let mut ws = WsSubscriber::new();
        let sub = make_sub("s1", EventType::NewBlock, "ws://localhost:8080");
        assert!(ws.subscribe(sub).is_ok());
        assert_eq!(ws.subscriptions.len(), 1);
    }

    #[test]
    fn test_subscribe_duplicate() {
        let mut ws = WsSubscriber::new();
        let sub = make_sub("s1", EventType::NewBlock, "ws://localhost:8080");
        ws.subscribe(sub).unwrap();
        let sub2 = make_sub("s1", EventType::PendingTx, "ws://localhost:8081");
        assert!(ws.subscribe(sub2).is_err());
    }

    #[test]
    fn test_unsubscribe_ok() {
        let mut ws = WsSubscriber::new();
        ws.subscribe(make_sub("s1", EventType::NewBlock, "ws://localhost:8080"))
            .unwrap();
        let removed = ws.unsubscribe("s1").unwrap();
        assert_eq!(removed.id, "s1");
        assert!(ws.subscriptions.is_empty());
    }

    #[test]
    fn test_unsubscribe_not_found() {
        let mut ws = WsSubscriber::new();
        assert!(ws.unsubscribe("missing").is_err());
    }

    #[test]
    fn test_pause_ok() {
        let mut ws = WsSubscriber::new();
        ws.subscribe(make_sub("s1", EventType::NewBlock, "ws://localhost:8080"))
            .unwrap();
        ws.pause("s1").unwrap();
        assert_eq!(ws.subscriptions["s1"].status, SubStatus::Paused);
    }

    #[test]
    fn test_pause_not_active() {
        let mut ws = WsSubscriber::new();
        ws.subscribe(make_sub("s1", EventType::NewBlock, "ws://localhost:8080"))
            .unwrap();
        ws.pause("s1").unwrap();
        assert!(ws.pause("s1").is_err());
    }

    #[test]
    fn test_resume_ok() {
        let mut ws = WsSubscriber::new();
        ws.subscribe(make_sub("s1", EventType::NewBlock, "ws://localhost:8080"))
            .unwrap();
        ws.pause("s1").unwrap();
        ws.resume("s1").unwrap();
        assert_eq!(ws.subscriptions["s1"].status, SubStatus::Active);
    }

    #[test]
    fn test_resume_not_paused() {
        let mut ws = WsSubscriber::new();
        ws.subscribe(make_sub("s1", EventType::NewBlock, "ws://localhost:8080"))
            .unwrap();
        assert!(ws.resume("s1").is_err());
    }

    #[test]
    fn test_record_event() {
        let mut ws = WsSubscriber::new();
        ws.subscribe(make_sub("s1", EventType::NewBlock, "ws://localhost:8080"))
            .unwrap();
        let event = make_event("s1", EventType::NewBlock);
        ws.record_event(event).unwrap();
        assert_eq!(ws.subscriptions["s1"].event_count, 1);
        assert!(ws.subscriptions["s1"].last_event.is_some());
        assert_eq!(ws.events.len(), 1);
    }

    #[test]
    fn test_record_event_not_found() {
        let mut ws = WsSubscriber::new();
        let event = make_event("missing", EventType::NewBlock);
        assert!(ws.record_event(event).is_err());
    }

    #[test]
    fn test_record_event_prune() {
        let mut ws = WsSubscriber::new();
        ws.max_events = 5;
        ws.subscribe(make_sub("s1", EventType::NewBlock, "ws://localhost:8080"))
            .unwrap();
        for _ in 0..8 {
            ws.record_event(make_event("s1", EventType::NewBlock))
                .unwrap();
        }
        assert_eq!(ws.events.len(), 5);
        assert_eq!(ws.subscriptions["s1"].event_count, 8);
    }

    #[test]
    fn test_record_error_increments() {
        let mut ws = WsSubscriber::new();
        ws.subscribe(make_sub("s1", EventType::NewBlock, "ws://localhost:8080"))
            .unwrap();
        ws.record_error("s1", "connection reset").unwrap();
        assert_eq!(ws.subscriptions["s1"].error_count, 1);
        assert_eq!(ws.subscriptions["s1"].status, SubStatus::Active);
    }

    #[test]
    fn test_record_error_disconnects_after_threshold() {
        let mut ws = WsSubscriber::new();
        ws.subscribe(make_sub("s1", EventType::NewBlock, "ws://localhost:8080"))
            .unwrap();
        for _ in 0..11 {
            ws.record_error("s1", "timeout").unwrap();
        }
        assert_eq!(ws.subscriptions["s1"].status, SubStatus::Disconnected);
    }

    #[test]
    fn test_matches_filters_eq() {
        let mut event = make_event("s1", EventType::TokenTransfer);
        event.payload.insert("token".to_string(), "EVAP".to_string());
        let filters = vec![EventFilter {
            field: "token".to_string(),
            op: FilterOp::Eq,
            value: "EVAP".to_string(),
        }];
        assert!(WsSubscriber::matches_filters(&event, &filters));
    }

    #[test]
    fn test_matches_filters_gt() {
        let mut event = make_event("s1", EventType::PriceUpdate);
        event.payload.insert("price".to_string(), "150".to_string());
        let filters = vec![EventFilter {
            field: "price".to_string(),
            op: FilterOp::Gt,
            value: "100".to_string(),
        }];
        assert!(WsSubscriber::matches_filters(&event, &filters));
    }

    #[test]
    fn test_matches_filters_contains() {
        let mut event = make_event("s1", EventType::ContractEvent);
        event
            .payload
            .insert("data".to_string(), "hello world".to_string());
        let filters = vec![EventFilter {
            field: "data".to_string(),
            op: FilterOp::Contains,
            value: "world".to_string(),
        }];
        assert!(WsSubscriber::matches_filters(&event, &filters));
    }

    #[test]
    fn test_matches_filters_missing_field() {
        let event = make_event("s1", EventType::NewBlock);
        let filters = vec![EventFilter {
            field: "missing".to_string(),
            op: FilterOp::Eq,
            value: "x".to_string(),
        }];
        assert!(!WsSubscriber::matches_filters(&event, &filters));
    }

    #[test]
    fn test_events_for_subscription() {
        let mut ws = WsSubscriber::new();
        ws.subscribe(make_sub("s1", EventType::NewBlock, "ws://localhost:8080"))
            .unwrap();
        ws.subscribe(make_sub("s2", EventType::PendingTx, "ws://localhost:8081"))
            .unwrap();
        ws.record_event(make_event("s1", EventType::NewBlock)).unwrap();
        ws.record_event(make_event("s2", EventType::PendingTx)).unwrap();
        ws.record_event(make_event("s1", EventType::NewBlock)).unwrap();
        let s1_events = ws.events_for_subscription("s1");
        assert_eq!(s1_events.len(), 2);
    }

    #[test]
    fn test_recent_events() {
        let mut ws = WsSubscriber::new();
        ws.subscribe(make_sub("s1", EventType::NewBlock, "ws://localhost:8080"))
            .unwrap();
        for _ in 0..5 {
            ws.record_event(make_event("s1", EventType::NewBlock)).unwrap();
        }
        let recent = ws.recent_events(3);
        assert_eq!(recent.len(), 3);
    }

    #[test]
    fn test_active_subscriptions() {
        let mut ws = WsSubscriber::new();
        ws.subscribe(make_sub("s1", EventType::NewBlock, "ws://localhost:8080"))
            .unwrap();
        ws.subscribe(make_sub("s2", EventType::PendingTx, "ws://localhost:8081"))
            .unwrap();
        ws.pause("s2").unwrap();
        assert_eq!(ws.active_subscriptions().len(), 1);
    }

    #[test]
    fn test_subscriptions_by_type() {
        let mut ws = WsSubscriber::new();
        ws.subscribe(make_sub("s1", EventType::NewBlock, "ws://localhost:8080"))
            .unwrap();
        ws.subscribe(make_sub("s2", EventType::NewBlock, "ws://localhost:8081"))
            .unwrap();
        ws.subscribe(make_sub("s3", EventType::PendingTx, "ws://localhost:8082"))
            .unwrap();
        let blocks = ws.subscriptions_by_type(&EventType::NewBlock);
        assert_eq!(blocks.len(), 2);
    }

    #[test]
    fn test_reconnect() {
        let mut ws = WsSubscriber::new();
        ws.subscribe(make_sub("s1", EventType::NewBlock, "ws://localhost:8080"))
            .unwrap();
        for _ in 0..12 {
            ws.record_error("s1", "timeout").unwrap();
        }
        assert_eq!(ws.subscriptions["s1"].status, SubStatus::Disconnected);
        ws.reconnect("s1").unwrap();
        assert_eq!(ws.subscriptions["s1"].status, SubStatus::Active);
        assert_eq!(ws.subscriptions["s1"].error_count, 0);
    }

    #[test]
    fn test_stats() {
        let mut ws = WsSubscriber::new();
        ws.subscribe(make_sub("s1", EventType::NewBlock, "ws://localhost:8080"))
            .unwrap();
        ws.subscribe(make_sub("s2", EventType::PendingTx, "ws://localhost:8081"))
            .unwrap();
        ws.pause("s2").unwrap();
        ws.record_event(make_event("s1", EventType::NewBlock)).unwrap();
        ws.record_error("s1", "minor").unwrap();
        let stats = ws.stats();
        assert_eq!(stats.total_subscriptions, 2);
        assert_eq!(stats.active, 1);
        assert_eq!(stats.paused, 1);
        assert_eq!(stats.total_events, 1);
        assert_eq!(stats.total_errors, 1);
    }

    #[test]
    fn test_save_and_load() {
        let path = temp_path("save_load");
        let mut ws = WsSubscriber::new();
        ws.subscribe(make_sub("s1", EventType::NewBlock, "ws://localhost:8080"))
            .unwrap();
        ws.record_event(make_event("s1", EventType::NewBlock)).unwrap();
        ws.save(&path).unwrap();

        let loaded = WsSubscriber::load(&path).unwrap();
        assert_eq!(loaded.subscriptions.len(), 1);
        assert_eq!(loaded.events.len(), 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_or_default_missing_file() {
        let path = temp_path("no_such_file");
        let ws = WsSubscriber::load_or_default(&path);
        assert!(ws.subscriptions.is_empty());
        assert_eq!(ws.max_events, 5000);
    }
}
