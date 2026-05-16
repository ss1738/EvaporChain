//! HTTP bid-intake server for the SFSV coordinator.
//!
//! Exposes one endpoint:
//!   POST /bid  — accept a `BidRequest`, enqueue it for the auctioneer.
//!
//! The server runs in a separate Tokio task and communicates with the
//! coordinator loop via an unbounded `tokio::sync::mpsc` channel.

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::info;

use crate::auctioneer::BidRequest;

pub type BidSender = mpsc::UnboundedSender<BidRequest>;

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
    if s.tx.send(req).is_err() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "coordinator shutting down" })),
        )
            .into_response();
    }
    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "status": "accepted" })),
    )
        .into_response()
}

/// Spawn the bid-intake HTTP server. Returns the `Receiver` end of the
/// bid channel for the coordinator loop to drain.
pub async fn spawn(port: u16) -> mpsc::UnboundedReceiver<BidRequest> {
    let (tx, rx) = mpsc::unbounded_channel::<BidRequest>();
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
