//! EvaporChain → Ethereum header relayer.
//!
//! Polls a chosen EvaporChain RPC endpoint for finalised block headers,
//! constructs the BLS commit-cert payload, and submits it to the
//! `EvaporHeaderInbox` contract on the configured Ethereum endpoint.
//!
//! Permissionless: anyone can run an instance. The receiving contract
//! verifies cryptographically; the relayer is purely a transport.
//!
//! See `ETHEREUM_BRIDGE_PLAN.md` Phase 3b for acceptance criteria.

mod chain_client;
mod config;
mod eth_client;
mod loop_runner;

use anyhow::Result;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let cfg = config::Config::from_env()?;
    info!(
        evaporchain_rpc = %cfg.evaporchain_rpc,
        ethereum_rpc    = %cfg.ethereum_rpc,
        inbox           = %cfg.inbox_address,
        "relayer starting"
    );

    let chain = chain_client::ChainClient::new(cfg.evaporchain_rpc.clone());
    let eth = eth_client::EthClient::new(&cfg).await?;

    loop_runner::run(chain, eth, cfg).await
}
