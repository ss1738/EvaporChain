//! Read-only client against the EvaporChain JSON API.
//!
//! Fetches finalised headers and the per-header commit-cert payload
//! the relayer needs to invoke `EvaporHeaderInbox.submitHeader`.
//!
//! Endpoints used (all exposed by `evaporchain-node` under `/api/bridge/`):
//!   - `GET /api/four_act` — current chain state (height, mmr_root, …)
//!   - `GET /api/bridge/headers/finalized?from=N` — list of finalised headers
//!   - `GET /api/bridge/headers/<height>/commit_cert` — BLS aggregate sig + bitmap
//!   - `GET /api/bridge/validators?epoch=N` — BLS-registered validator set

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinalisedHeader {
    pub height: u64,
    pub block_hash: String,    // 0x… 32 bytes
    pub state_root: String,    // 0x… 32 bytes
    pub mmr_root: String,      // 0x… 32 bytes
    pub epoch: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitCertResponse {
    /// Hex of the BLS-G2 aggregate signature in EIP-2537 uncompressed form (256 bytes).
    pub agg_signature_uncompressed: String,
    /// Hex of the LSB-first packed bitmap.
    pub signed_bitmap: String,
    /// One uncompressed G1 point per validator, concatenated, hex-encoded.
    pub prev_pubkeys_uncompressed: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorEntry {
    pub pubkey_compressed: String, // 0x… 48 bytes
    pub stake: u128,
}

pub struct ChainClient {
    base: String,
    http: reqwest::Client,
}

impl ChainClient {
    pub fn new(base: impl Into<String>) -> Self {
        Self {
            base: base.into(),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("reqwest client"),
        }
    }

    pub async fn finalised_headers_since(&self, from: u64) -> Result<Vec<FinalisedHeader>> {
        let url = format!("{}/api/bridge/headers/finalized?from={}", self.base, from);
        let resp: Vec<FinalisedHeader> = self.http.get(&url).send().await?.error_for_status()?.json().await?;
        Ok(resp)
    }

    pub async fn commit_cert(&self, height: u64) -> Result<CommitCertResponse> {
        let url = format!("{}/api/bridge/headers/{}/commit_cert", self.base, height);
        let resp: CommitCertResponse = self.http.get(&url).send().await?.error_for_status()?.json().await?;
        Ok(resp)
    }

    pub async fn validators_at(&self, epoch: u64) -> Result<Vec<ValidatorEntry>> {
        let url = format!("{}/api/bridge/validators?epoch={}", self.base, epoch);
        let resp: Vec<ValidatorEntry> = self.http.get(&url).send().await?.error_for_status()?.json().await?;
        Ok(resp)
    }
}
