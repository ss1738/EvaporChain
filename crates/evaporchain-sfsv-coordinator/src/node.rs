//! Thin HTTP client wrapping the EvaporChain node API.
//!
//! Only the subset of endpoints the coordinator needs:
//!   GET  /api/scripts              — discover deployed vault contracts
//!   GET  /api/script/:id           — fetch contract state (listing fields)
//!   GET  /api/status               — current epoch
//!   GET  /api/tx/:hash             — poll tx finality
//!   POST /api/tx/call-script       — submit record_sale
//!   POST /api/auth/login           — obtain session token (if needed)

use serde::Deserialize;
use serde_json::{json, Value};
use thiserror::Error;

use crate::config::Config;

#[derive(Debug, Error)]
pub enum NodeError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("unexpected response: {0}")]
    Parse(String),
    #[error("tx timeout — {tx_hash} not finalised in time")]
    Timeout { tx_hash: String },
    #[error("tx rejected: {0}")]
    TxRejected(String),
    #[error("login failed: {0}")]
    LoginFailed(String),
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScriptSummary {
    pub id: u64,
    pub name: String,
    pub methods: Vec<String>,
    pub evaporated: bool,
}

#[derive(Debug, Clone)]
pub struct NodeClient {
    http: reqwest::Client,
    base: String,
    token: String,
}

impl NodeClient {
    pub fn new(cfg: &Config) -> Self {
        Self {
            http: reqwest::Client::new(),
            base: cfg.node_url.clone(),
            token: cfg.auth_token.clone(),
        }
    }

    /// Obtain a session token by logging in. Overwrites the stored token.
    pub async fn login(&mut self, email: &str, password: &str) -> Result<(), NodeError> {
        let resp: Value = self
            .http
            .post(format!("{}/api/auth/login", self.base))
            .json(&json!({ "email": email, "password": password }))
            .send()
            .await?
            .json()
            .await?;
        let token = resp
            .get("token")
            .and_then(|t| t.as_str())
            .ok_or_else(|| NodeError::LoginFailed(resp.to_string()))?;
        self.token = token.to_owned();
        Ok(())
    }

    /// Current consensus epoch from `/api/status`.
    pub async fn epoch(&self) -> Result<u64, NodeError> {
        let resp: Value = self
            .http
            .get(format!("{}/api/status", self.base))
            .send()
            .await?
            .json()
            .await?;
        resp.get("epoch")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| NodeError::Parse("missing epoch in /api/status".into()))
    }

    /// All deployed scripts (contracts). Filters to live (non-evaporated) ones.
    pub async fn list_scripts(&self) -> Result<Vec<ScriptSummary>, NodeError> {
        let resp: Value = self
            .http
            .get(format!("{}/api/scripts", self.base))
            .send()
            .await?
            .json()
            .await?;
        let arr = resp
            .get("scripts")
            .and_then(|s| s.as_array())
            .cloned()
            .unwrap_or_default();
        let summaries: Vec<ScriptSummary> = arr
            .into_iter()
            .filter_map(|v| serde_json::from_value(v).ok())
            .filter(|s: &ScriptSummary| !s.evaporated)
            .collect();
        Ok(summaries)
    }

    /// Full script detail including on-chain state (listing fields).
    pub async fn get_script(&self, id: u64) -> Result<Value, NodeError> {
        let resp = self
            .http
            .get(format!("{}/api/script/{}", self.base, id))
            .send()
            .await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(NodeError::Parse(format!("script {id} not found")));
        }
        Ok(resp.json().await?)
    }

    /// Submit a `call-script` transaction. Returns the tx hash on success.
    pub async fn call_script(
        &self,
        caller_byte: u8,
        contract_id: u64,
        method: &str,
        args: Value,
        epoch: u64,
    ) -> Result<String, NodeError> {
        let mut req = self.http.post(format!("{}/api/tx/call-script", self.base));
        if !self.token.is_empty() {
            req = req.bearer_auth(&self.token);
        }
        let body = json!({
            "caller": caller_byte,
            "contract_id": contract_id,
            "method": method,
            "args": args,
            "epoch": epoch,
        });
        let resp: Value = req.json(&body).send().await?.json().await?;
        if resp.get("success").and_then(|v| v.as_bool()) == Some(true) {
            let hash = resp
                .get("tx_hash")
                .and_then(|v| v.as_str())
                .ok_or_else(|| NodeError::Parse("missing tx_hash".into()))?;
            Ok(hash.to_owned())
        } else {
            Err(NodeError::Parse(
                resp.get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown error")
                    .to_owned(),
            ))
        }
    }

    /// Poll `/api/tx/:hash` until finalised or timeout. Returns final state string.
    pub async fn wait_finalised(&self, tx_hash: &str) -> Result<String, NodeError> {
        let deadline = std::time::Instant::now()
            + std::time::Duration::from_millis(crate::config::FINALITY_TIMEOUT_MS);
        loop {
            if std::time::Instant::now() > deadline {
                return Err(NodeError::Timeout {
                    tx_hash: tx_hash.to_owned(),
                });
            }
            let resp: Value = self
                .http
                .get(format!("{}/api/tx/{}", self.base, tx_hash))
                .send()
                .await?
                .json()
                .await?;
            let state = resp
                .get("state")
                .or_else(|| resp.get("status"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_owned();
            match state.as_str() {
                // "included" is omitted — it means the tx is in a block
                // but not yet BFT-finalised; a reorg can still remove it.
                // Only treat consensus-final states as done (audit F5 2026-05-18).
                "finalised" | "finalized" | "committed" => return Ok(state),
                "rejected" => {
                    return Err(NodeError::TxRejected(resp.to_string()));
                }
                _ => {}
            }
            tokio::time::sleep(std::time::Duration::from_millis(
                crate::config::FINALITY_POLL_MS,
            ))
            .await;
        }
    }
}
