//! Main relayer event loop. Pull finalised headers from EvaporChain,
//! push them to Ethereum in order, sleep, repeat.
//!
//! Phase 3b scaffold. Submission is a no-op until `eth_client::submit_header`
//! is wired against the alloy contract binding (see TODO there).

use anyhow::Result;
use std::time::Duration;
use tracing::{info, warn};

use crate::chain_client::ChainClient;
use crate::config::Config;
use crate::eth_client::EthClient;

pub async fn run(chain: ChainClient, eth: EthClient, cfg: Config) -> Result<()> {
    let mut next_height = match cfg.start_height_override {
        Some(h) => h,
        None => eth.latest_height().await? + 1,
    };

    info!(starting_height = next_height, "relayer loop starting");

    loop {
        let headers = match chain.finalised_headers_since(next_height).await {
            Ok(h) => h,
            Err(e) => {
                warn!(err = %e, "failed to fetch finalised headers; will retry");
                tokio::time::sleep(Duration::from_secs(cfg.poll_interval_secs)).await;
                continue;
            }
        };

        for h in headers {
            info!(
                height = h.height,
                block_hash = %h.block_hash,
                "submitting header"
            );

            // Fetch commit-cert for this header.
            let cert = match chain.commit_cert(h.height).await {
                Ok(c) => c,
                Err(e) => {
                    warn!(err = %e, height = h.height, "failed to fetch commit-cert; skipping");
                    continue;
                }
            };

            let validators = match chain.validators_at(h.epoch).await {
                Ok(v) => v,
                Err(e) => {
                    warn!(err = %e, epoch = h.epoch, "failed to fetch validator set; skipping");
                    continue;
                }
            };

            let validators_compressed: Vec<(Vec<u8>, u128)> = validators
                .into_iter()
                .map(|v| (hex_decode_0x(&v.pubkey_compressed), v.stake))
                .collect();

            // Try submit. submit_header currently returns an Err under the
            // Phase 3b scaffold — that's expected; loop logs and continues.
            match eth
                .submit_header(
                    h.height,
                    parse32(&h.block_hash),
                    parse32(&h.state_root),
                    parse32(&h.mmr_root),
                    h.epoch,
                    validators_compressed,
                    hex_decode_0x(&cert.prev_pubkeys_uncompressed),
                    hex_decode_0x(&cert.signed_bitmap),
                    hex_decode_0x(&cert.agg_signature_uncompressed),
                )
                .await
            {
                Ok(tx) => info!(height = h.height, tx, "submitted"),
                Err(e) => warn!(height = h.height, err = %e, "submission failed (Phase 3b stub)"),
            }

            next_height = h.height + 1;
        }

        tokio::time::sleep(Duration::from_secs(cfg.poll_interval_secs)).await;
    }
}

fn hex_decode_0x(s: &str) -> Vec<u8> {
    let stripped = s.trim_start_matches("0x");
    hex::decode(stripped).unwrap_or_default()
}

fn parse32(s: &str) -> [u8; 32] {
    let v = hex_decode_0x(s);
    let mut out = [0u8; 32];
    let take = v.len().min(32);
    out[..take].copy_from_slice(&v[..take]);
    out
}
