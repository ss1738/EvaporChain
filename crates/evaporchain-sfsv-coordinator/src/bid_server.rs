//! HTTP bid-intake server for the SFSV coordinator.
//!
//! Exposes one endpoint:
//!   POST /bid  — accept a `BidRequest`, enqueue it for the auctioneer.
//!
//! The server runs in a separate Tokio task and communicates with the
//! coordinator loop via a **bounded** mpsc channel (capacity MAX_BID_QUEUE).
//! When the queue is full the server returns 429 Too Many Requests so
//! callers back-off rather than the process silently accumulating unbounded
//! memory (audit 2026-05-18 F1).

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::auctioneer::BidRequest;

/// Maximum number of unprocessed bids buffered in the channel.
/// The coordinator drains the queue every tick (~1s); 1024 gives
/// ~17 min of headroom at one bid per second before back-pressure fires.
pub const MAX_BID_QUEUE: usize = 1_024;

pub type BidSender = mpsc::Sender<BidRequest>;

#[derive(Clone)]
struct ServerState {
    tx: Arc<BidSender>,
}

async fn handle_bid(
    State(s): State<ServerState>,
    Json(req): Json<BidRequest>,
) -> impl IntoResponse {
    info!(
        contract_id = req.contract_id,
        bidder = %req.bidder_hex,
        max_price = req.max_price,
        "bid received via HTTP"
    );
    match s.tx.try_send(req) {
        Ok(()) => (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({ "status": "accepted" })),
        )
            .into_response(),
        Err(mpsc::error::TrySendError::Full(_)) => {
            warn!("bid queue full — returning 429");
            (
                StatusCode::TOO_MANY_REQUESTS,
                Json(serde_json::json!({ "error": "bid queue full, retry later" })),
            )
                .into_response()
        }
        Err(mpsc::error::TrySendError::Closed(_)) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "coordinator shutting down" })),
        )
            .into_response(),
    }
}

/// Spawn the bid-intake HTTP server. Returns the `Receiver` end of the
/// bid channel for the coordinator loop to drain.
pub async fn spawn(port: u16) -> mpsc::Receiver<BidRequest> {
    let (tx, rx) = mpsc::channel::<BidRequest>(MAX_BID_QUEUE);
    let state = ServerState { tx: Arc::new(tx) };
    let app = Router::new()
        .route("/bid", post(handle_bid))
        .with_state(state);
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    info!(%addr, "SFSV bid-intake server starting");
    tokio::spawn(async move {
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .expect("bid-intake server bind failed");
        axum::serve(listener, app)
            .await
            .expect("bid-intake server error");
    });
    rx
}
