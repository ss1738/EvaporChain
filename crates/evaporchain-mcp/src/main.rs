//! EvaporChain MCP Server — Model Context Protocol for AI agents
//!
//! Implements the MCP specification over stdio (JSON-RPC 2.0).
//! Connects to a running EvaporChain node via HTTP API.
//!
//! 26 Tools · 13 Resources · 6 Prompts
//!
//! Start: evaporchain-mcp [--node-url http://37.27.1.1:8080]

mod prompts;
mod protocol;
mod resources;
mod tools;

use std::env;
use tokio::io::{self, AsyncBufReadExt, BufReader};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let node_url = env::args()
        .skip_while(|a| a != "--node-url")
        .nth(1)
        .unwrap_or_else(|| "http://37.27.1.1:8080".to_string());

    let client = reqwest::Client::new();
    let ctx = protocol::Context { node_url, client };

    let stdin = BufReader::new(io::stdin());
    let mut lines = stdin.lines();

    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        let response = protocol::handle_message(&ctx, &line).await;
        if let Some(resp) = response {
            let out = serde_json::to_string(&resp)?;
            println!("{}", out);
        }
    }

    Ok(())
}
