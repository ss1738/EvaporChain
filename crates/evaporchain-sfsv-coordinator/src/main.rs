//! SFSV off-chain coordinator binary.
//!
//! Architecture:
//!
//! ```text
//!   ┌──────────────────────────────────────────────────────┐
//!   │  Bid-intake HTTP server  (port 9100)                 │
//!   │  POST /bid → BidRequest ─────────────────────────┐   │
//!   └──────────────────────────────────────────────────│───┘
//!                                                       │ mpsc channel
//!   ┌──────────────────────────────────────────────────▼───┐
//!   │  Coordinator loop  (every 2 s)                        │
//!   │  1. poll /api/scripts  → discover active listings     │
//!   │  2. drain bid channel  → Auctioneer::submit_bid       │
//!   │  3. Auctioneer::try_clear_all(epoch_now)              │
//!   │  4. for each ClearResult → submit record_sale TX      │
//!   │  5. wait for TX finality → log outcome               │
//!   └───────────────────────────────────────────────────────┘
//! ```
//!
//! The on-chain `record_sale` enforces the vault state machine guards;
//! the off-chain SDDC clear only decides the winner.  A clear cannot
//! bypass a released or unlisted vault — the chain rejects it.
//!
//! INVENTION_STACK §A5.2: SFSV coordinator.

use evaporchain_sfsv_coordinator::{
    auctioneer::Auctioneer,
    bid_server,
    config::{Config, POLL_INTERVAL_MS},
    node::NodeClient,
    poller,
};
use serde_json::json;
use tracing::{error, info, warn};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("SFSV_LOG")
                .unwrap_or_else(|_| "info".parse().unwrap()),
        )
        .init();

    let cfg = Config::from_env();
    info!(
        node = %cfg.node_url,
        bid_port = cfg.bid_server_port,
        "SFSV coordinator starting"
    );

    // Start bid-intake HTTP server; get the channel receiver.
    let mut bid_rx = bid_server::spawn(cfg.bid_server_port).await;

    let client = NodeClient::new(&cfg);
    let mut auctioneer = Auctioneer::new();

    loop {
        // ── 1. Poll for active listings ───────────────────────────────
        let listings = poller::discover_listings(&client).await;
        for (contract_id, vault, auction) in listings {
            if !auctioneer.is_tracked(contract_id) {
                let _ = auctioneer.register_listing(contract_id, vault, auction);
            }
        }

        // ── 2. Drain incoming bids ────────────────────────────────────
        while let Ok(bid_req) = bid_rx.try_recv() {
            if let Err(e) = auctioneer.submit_bid(&bid_req) {
                warn!(error = %e, "bid rejected by auctioneer");
            }
        }

        // ── 3. Try to clear open auctions ─────────────────────────────
        let epoch_now = match client.epoch().await {
            Ok(e) => e,
            Err(e) => {
                warn!(error = %e, "failed to fetch epoch; skipping clear tick");
                tokio::time::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS)).await;
                continue;
            }
        };
        let clears = auctioneer.try_clear_all(epoch_now);

        // ── 4 + 5. Submit record_sale for each winner ─────────────────
        for clear in clears {
            let winner_hex = hex::encode(clear.winner);
            info!(
                contract_id = clear.contract_id,
                winner = %winner_hex,
                price_paid = clear.price_paid,
                epoch = clear.epoch,
                "submitting record_sale"
            );

            // `.es` record_sale signature: record_sale(winner_addr: Address)
            let args = json!([
                { "Address": clear.winner.to_vec() }
            ]);

            let submit_result = client
                .call_script(
                    cfg.caller_byte,
                    clear.contract_id,
                    "record_sale",
                    args,
                    clear.epoch,
                )
                .await;

            match submit_result {
                Ok(tx_hash) => {
                    info!(
                        contract_id = clear.contract_id,
                        %tx_hash,
                        "record_sale TX submitted — waiting for finality"
                    );
                    match client.wait_finalised(&tx_hash).await {
                        Ok(state) => {
                            info!(
                                contract_id = clear.contract_id,
                                %tx_hash,
                                %state,
                                winner = %winner_hex,
                                "record_sale FINALISED ✓"
                            );
                        }
                        Err(e) => {
                            error!(
                                contract_id = clear.contract_id,
                                %tx_hash,
                                error = %e,
                                "record_sale finality error"
                            );
                        }
                    }
                }
                Err(e) => {
                    error!(
                        contract_id = clear.contract_id,
                        error = %e,
                        "record_sale TX submission failed"
                    );
                }
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS)).await;
    }
}
