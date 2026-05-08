//! Relayer configuration. Read from environment variables so the
//! binary stays a single static artifact deployable anywhere.

use anyhow::{Context, Result};

/// All knobs the relayer needs at startup.
pub struct Config {
    /// HTTP base URL of an EvaporChain node API (e.g. `http://100.119.53.101:8080`).
    pub evaporchain_rpc: String,
    /// HTTP RPC URL of the Ethereum endpoint (Anvil/Sepolia/Mainnet).
    pub ethereum_rpc: String,
    /// `0x…` address of the deployed `EvaporHeaderInbox` contract.
    pub inbox_address: String,
    /// 0x-prefixed hex of the relayer signer private key.
    pub signer_private_key: String,
    /// How often to poll EvaporChain for new finalised headers.
    pub poll_interval_secs: u64,
    /// Highest already-finalised height the relayer should treat as the
    /// latest known on the Ethereum side. The relayer reads this from the
    /// inbox before starting.
    pub start_height_override: Option<u64>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            evaporchain_rpc: env_or("EVAPORCHAIN_RPC", "http://127.0.0.1:8080")?,
            ethereum_rpc: env_or("ETHEREUM_RPC", "http://127.0.0.1:8545")?,
            inbox_address: env("INBOX_ADDRESS")
                .context("INBOX_ADDRESS must be set (0x… of EvaporHeaderInbox deployment)")?,
            signer_private_key: env("RELAYER_PRIVATE_KEY")
                .context("RELAYER_PRIVATE_KEY must be set (0x-prefixed)")?,
            poll_interval_secs: env_or("POLL_INTERVAL_SECS", "5")?.parse().unwrap_or(5),
            start_height_override: std::env::var("START_HEIGHT_OVERRIDE")
                .ok()
                .and_then(|s| s.parse().ok()),
        })
    }
}

fn env(key: &str) -> Result<String> {
    std::env::var(key).map_err(|_| anyhow::anyhow!("env var {key} not set"))
}

fn env_or(key: &str, default: &str) -> Result<String> {
    Ok(std::env::var(key).unwrap_or_else(|_| default.to_string()))
}
