use axum::extract::ws::{Message, WebSocket};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

// ──────────────────────────── Event Types ───────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
#[allow(dead_code)]
pub enum WsEvent {
    #[serde(rename = "new_block")]
    NewBlock {
        number: u64,
        epoch: u64,
        tx_count: usize,
        timestamp: u64,
        state_root: String,
        producer: Option<String>,
    },
    #[serde(rename = "new_transaction")]
    NewTransaction {
        hash: String,
        tx_type: String,
        from: String,
        to: Option<String>,
        amount: Option<u64>,
    },
    #[serde(rename = "evaporation")]
    Evaporation {
        object_id: String,
        energy: f64,
        block_number: u64,
    },
    #[serde(rename = "grace_period")]
    GracePeriod {
        object_id: String,
        remaining_energy: f64,
        block_number: u64,
    },
    #[serde(rename = "chain_event")]
    ChainEvent {
        event_type: String,
        message: String,
        epoch: u64,
        timestamp_ms: u64,
    },
    #[serde(rename = "peer_update")]
    PeerUpdate { connected: usize },
    #[serde(rename = "contract_event")]
    ContractLog {
        contract_id: u64,
        block_number: u64,
        event_name: String,
        topics: Vec<String>,
        data: Vec<String>,
    },
}

// ──────────────────────────── Subscription Filter ───────────────────────

#[derive(Debug, Deserialize)]
pub struct WsSubscribeParams {
    /// Comma-separated event types to subscribe to.
    /// Options: blocks, transactions, evaporations, events, peers, contract_events, all
    /// Default: all
    pub subscribe: Option<String>,
}

#[derive(Debug, Clone)]
struct SubscriptionFilter {
    blocks: bool,
    transactions: bool,
    evaporations: bool,
    events: bool,
    peers: bool,
    contract_events: bool,
}

/// All topic names recognised by the WebSocket subscribe parser. A
/// subscribe param containing any other token is logged as a typo so
/// clients aren't silently filtered to nothing — see Network/N10
/// (re-audit 2026-05-02).
const KNOWN_WS_TOPICS: &[&str] = &[
    "blocks",
    "transactions",
    "evaporations",
    "events",
    "peers",
    "contract_events",
];

impl SubscriptionFilter {
    fn from_param(param: &Option<String>) -> Self {
        match param {
            None => Self::all(),
            Some(s) if s == "all" => Self::all(),
            Some(s) => {
                let topics: Vec<&str> = s.split(',').map(|t| t.trim()).collect();
                for t in &topics {
                    if !t.is_empty() && !KNOWN_WS_TOPICS.contains(t) {
                        warn!(
                            "WebSocket subscribe: unknown topic '{}' (known: {}). Client will receive nothing for that token.",
                            t,
                            KNOWN_WS_TOPICS.join(",")
                        );
                    }
                }
                Self {
                    blocks: topics.contains(&"blocks"),
                    transactions: topics.contains(&"transactions"),
                    evaporations: topics.contains(&"evaporations"),
                    events: topics.contains(&"events"),
                    peers: topics.contains(&"peers"),
                    contract_events: topics.contains(&"contract_events"),
                }
            }
        }
    }

    fn all() -> Self {
        Self {
            blocks: true,
            transactions: true,
            evaporations: true,
            events: true,
            peers: true,
            contract_events: true,
        }
    }

    fn matches(&self, event: &WsEvent) -> bool {
        match event {
            WsEvent::NewBlock { .. } => self.blocks,
            WsEvent::NewTransaction { .. } => self.transactions,
            WsEvent::Evaporation { .. } | WsEvent::GracePeriod { .. } => self.evaporations,
            WsEvent::ChainEvent { .. } => self.events,
            WsEvent::PeerUpdate { .. } => self.peers,
            WsEvent::ContractLog { .. } => self.contract_events,
        }
    }
}

// ──────────────────────────── Broadcaster ───────────────────────────────

/// M11 (audit 2026-05-02): cap concurrent WebSocket subscribers globally.
/// Without this an attacker can open thousands of sockets, each spawning
/// a tokio task and consuming a `broadcast::Receiver` slot. Default is
/// 4096 subscribers — far above legitimate UI / wallet load, well below
/// kernel FD limits. Override via `WsBroadcaster::new_with_cap`.
pub const DEFAULT_MAX_WS_SUBSCRIBERS: usize = 4096;

pub struct WsBroadcaster {
    tx: broadcast::Sender<WsEvent>,
    max_subscribers: usize,
}

impl WsBroadcaster {
    pub fn new(capacity: usize) -> Self {
        Self::new_with_cap(capacity, DEFAULT_MAX_WS_SUBSCRIBERS)
    }

    pub fn new_with_cap(capacity: usize, max_subscribers: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self {
            tx,
            max_subscribers,
        }
    }

    pub fn publish(&self, event: WsEvent) {
        let _ = self.tx.send(event);
    }

    #[allow(dead_code)]
    pub fn subscribe(&self) -> broadcast::Receiver<WsEvent> {
        self.tx.subscribe()
    }

    pub fn try_subscribe(&self) -> Option<broadcast::Receiver<WsEvent>> {
        if self.tx.receiver_count() >= self.max_subscribers {
            return None;
        }
        Some(self.tx.subscribe())
    }

    pub fn subscriber_count(&self) -> usize {
        self.tx.receiver_count()
    }

    pub fn max_subscribers(&self) -> usize {
        self.max_subscribers
    }
}

// ──────────────────────────── Handler ───────────────────────────────────

pub async fn handle_ws_connection(
    socket: WebSocket,
    broadcaster: Arc<WsBroadcaster>,
    params: WsSubscribeParams,
) {
    let filter = SubscriptionFilter::from_param(&params.subscribe);
    handle_ws(socket, broadcaster, filter).await
}

async fn handle_ws(socket: WebSocket, broadcaster: Arc<WsBroadcaster>, filter: SubscriptionFilter) {
    let (mut sender, mut receiver) = socket.split();

    // M11 (audit 2026-05-02): refuse new subscribers past the cap so a
    // socket-flood attack can't exhaust task / receiver slots.
    let mut rx = match broadcaster.try_subscribe() {
        Some(rx) => rx,
        None => {
            let busy = serde_json::json!({
                "type": "error",
                "message": format!(
                    "WebSocket subscriber cap reached ({}); try again later",
                    broadcaster.max_subscribers()
                ),
            });
            let _ = sender
                .send(Message::Text(serde_json::to_string(&busy).unwrap()))
                .await;
            let _ = sender.close().await;
            return;
        }
    };

    let welcome = serde_json::json!({
        "type": "connected",
        "message": "EvaporChain WebSocket v1",
        "subscribers": broadcaster.subscriber_count(),
    });
    if sender
        .send(Message::Text(serde_json::to_string(&welcome).unwrap()))
        .await
        .is_err()
    {
        return;
    }

    let filter_clone = filter.clone();
    let send_task = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if !filter_clone.matches(&event) {
                        continue;
                    }
                    let json = match serde_json::to_string(&event) {
                        Ok(j) => j,
                        Err(_) => continue,
                    };
                    if sender.send(Message::Text(json)).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!("WebSocket client lagged by {n} messages");
                    let lag_msg = serde_json::json!({
                        "type": "warning",
                        "message": format!("Lagged by {n} events — some events were dropped"),
                    });
                    let _ = sender
                        .send(Message::Text(serde_json::to_string(&lag_msg).unwrap()))
                        .await;
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    let recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            match msg {
                Message::Close(_) => break,
                Message::Ping(_data) => {
                    debug!("WebSocket ping received");
                }
                Message::Text(text) => {
                    debug!("WebSocket text from client: {text}");
                }
                _ => {}
            }
        }
    });

    tokio::select! {
        _ = send_task => {},
        _ = recv_task => {},
    }

    info!("WebSocket connection closed");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_all_matches_everything() {
        let f = SubscriptionFilter::all();
        assert!(f.matches(&WsEvent::NewBlock {
            number: 1,
            epoch: 1,
            tx_count: 0,
            timestamp: 0,
            state_root: String::new(),
            producer: None,
        }));
        assert!(f.matches(&WsEvent::PeerUpdate { connected: 3 }));
    }

    #[test]
    fn filter_specific_topics() {
        let f = SubscriptionFilter::from_param(&Some("blocks,peers".to_string()));
        assert!(f.matches(&WsEvent::NewBlock {
            number: 1,
            epoch: 1,
            tx_count: 0,
            timestamp: 0,
            state_root: String::new(),
            producer: None,
        }));
        assert!(f.matches(&WsEvent::PeerUpdate { connected: 3 }));
        assert!(!f.matches(&WsEvent::NewTransaction {
            hash: String::new(),
            tx_type: String::new(),
            from: String::new(),
            to: None,
            amount: None,
        }));
        assert!(!f.matches(&WsEvent::Evaporation {
            object_id: String::new(),
            energy: 0.0,
            block_number: 0,
        }));
    }

    #[test]
    fn broadcaster_publish_subscribe() {
        let b = WsBroadcaster::new(16);
        let mut rx = b.subscribe();
        b.publish(WsEvent::PeerUpdate { connected: 5 });
        let event = rx.try_recv().unwrap();
        match event {
            WsEvent::PeerUpdate { connected } => assert_eq!(connected, 5),
            _ => panic!("wrong event type"),
        }
    }

    #[test]
    fn broadcaster_no_subscribers_doesnt_panic() {
        let b = WsBroadcaster::new(16);
        b.publish(WsEvent::PeerUpdate { connected: 0 });
    }

    // ─── Coverage push (2026-05-16): lift ws.rs from ~47% ───

    /// `subscribe = None` ⇒ all topics enabled (default behaviour).
    #[test]
    fn filter_none_subscribe_param_means_all() {
        let f = SubscriptionFilter::from_param(&None);
        assert!(f.blocks);
        assert!(f.transactions);
        assert!(f.evaporations);
        assert!(f.events);
        assert!(f.peers);
        assert!(f.contract_events);
    }

    /// Empty string ⇒ no topics enabled — caller learns by receiving nothing.
    #[test]
    fn filter_empty_subscribe_string_enables_nothing() {
        let f = SubscriptionFilter::from_param(&Some(String::new()));
        assert!(!f.blocks);
        assert!(!f.transactions);
        assert!(!f.evaporations);
        assert!(!f.events);
        assert!(!f.peers);
        assert!(!f.contract_events);
    }

    /// Unknown topic tokens are logged but don't enable anything.
    #[test]
    fn filter_unknown_topic_token_enables_nothing() {
        let f = SubscriptionFilter::from_param(&Some("typo_in_topic".to_string()));
        assert!(!f.blocks);
        assert!(!f.transactions);
    }

    /// `evaporations` covers BOTH Evaporation AND GracePeriod events
    /// (per the match arm at line 138).
    #[test]
    fn filter_evaporations_includes_grace_period() {
        let f = SubscriptionFilter::from_param(&Some("evaporations".to_string()));
        assert!(f.matches(&WsEvent::Evaporation {
            object_id: String::new(),
            energy: 0.0,
            block_number: 0,
        }));
        assert!(f.matches(&WsEvent::GracePeriod {
            object_id: String::new(),
            remaining_energy: 0.0,
            block_number: 0,
        }));
    }

    /// `contract_events` filter gates the ContractLog arm.
    #[test]
    fn filter_contract_events_gates_contract_log() {
        let yes = SubscriptionFilter::from_param(&Some("contract_events".to_string()));
        let no = SubscriptionFilter::from_param(&Some("blocks".to_string()));
        let log = WsEvent::ContractLog {
            contract_id: 1,
            block_number: 0,
            event_name: "test".into(),
            topics: vec![],
            data: vec![],
        };
        assert!(yes.matches(&log));
        assert!(!no.matches(&log));
    }

    /// Whitespace around comma-separated tokens is stripped.
    #[test]
    fn filter_trims_whitespace_around_tokens() {
        let f = SubscriptionFilter::from_param(&Some(" blocks , peers ".to_string()));
        assert!(f.blocks);
        assert!(f.peers);
        assert!(!f.transactions);
    }

    /// M11 (audit 2026-05-02): `try_subscribe` enforces the global
    /// concurrent-subscriber cap.  At-cap requests return None
    /// without crashing.
    #[test]
    fn broadcaster_try_subscribe_respects_cap() {
        let b = WsBroadcaster::new_with_cap(16, 2);
        let r1 = b.try_subscribe();
        let r2 = b.try_subscribe();
        let r3 = b.try_subscribe(); // at cap — must fail
        assert!(r1.is_some());
        assert!(r2.is_some());
        assert!(r3.is_none(), "M11 cap: third subscribe must be denied");
        assert_eq!(b.subscriber_count(), 2);
        assert_eq!(b.max_subscribers(), 2);
    }

    /// Dropping a subscriber frees a slot for a new one (cap is a
    /// running count, not a lifetime quota).
    #[test]
    fn broadcaster_drop_frees_subscriber_slot() {
        let b = WsBroadcaster::new_with_cap(16, 1);
        {
            let _rx = b.try_subscribe().unwrap();
            assert!(b.try_subscribe().is_none());
        } // _rx dropped
        assert!(b.try_subscribe().is_some());
    }

    /// `WsEvent` serializes with the discriminant tag "type".
    #[test]
    fn event_serialization_includes_type_tag() {
        let ev = WsEvent::NewBlock {
            number: 42,
            epoch: 7,
            tx_count: 3,
            timestamp: 1000,
            state_root: "0xdeadbeef".into(),
            producer: Some("validator-1".into()),
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains("\"type\":\"new_block\""), "wrong tag: {json}");
        assert!(json.contains("\"number\":42"));
    }

    /// All 7 WsEvent variants serialize without panicking and carry
    /// distinct `type` tags.
    #[test]
    fn all_event_variants_serialize_distinct_type_tags() {
        let events = vec![
            WsEvent::NewBlock {
                number: 1,
                epoch: 1,
                tx_count: 0,
                timestamp: 0,
                state_root: String::new(),
                producer: None,
            },
            WsEvent::NewTransaction {
                hash: String::new(),
                tx_type: String::new(),
                from: String::new(),
                to: None,
                amount: None,
            },
            WsEvent::Evaporation {
                object_id: String::new(),
                energy: 0.0,
                block_number: 0,
            },
            WsEvent::GracePeriod {
                object_id: String::new(),
                remaining_energy: 0.0,
                block_number: 0,
            },
            WsEvent::ChainEvent {
                event_type: String::new(),
                message: String::new(),
                epoch: 0,
                timestamp_ms: 0,
            },
            WsEvent::PeerUpdate { connected: 0 },
            WsEvent::ContractLog {
                contract_id: 0,
                block_number: 0,
                event_name: String::new(),
                topics: vec![],
                data: vec![],
            },
        ];
        let mut seen_tags = std::collections::HashSet::new();
        for ev in events {
            let v: serde_json::Value = serde_json::to_value(&ev).unwrap();
            let tag = v.get("type").and_then(|t| t.as_str()).unwrap().to_string();
            assert!(seen_tags.insert(tag.clone()), "duplicate tag: {tag}");
        }
        assert_eq!(seen_tags.len(), 7);
    }
}
