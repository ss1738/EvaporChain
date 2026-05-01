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

impl SubscriptionFilter {
    fn from_param(param: &Option<String>) -> Self {
        match param {
            None => Self::all(),
            Some(s) if s == "all" => Self::all(),
            Some(s) => {
                let topics: Vec<&str> = s.split(',').map(|t| t.trim()).collect();
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

pub struct WsBroadcaster {
    tx: broadcast::Sender<WsEvent>,
}

impl WsBroadcaster {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    pub fn publish(&self, event: WsEvent) {
        let _ = self.tx.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<WsEvent> {
        self.tx.subscribe()
    }

    pub fn subscriber_count(&self) -> usize {
        self.tx.receiver_count()
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
    let mut rx = broadcaster.subscribe();

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
}
