//! EvaporChain MCP Server — Model Context Protocol for AI agents
//!
//! Implements the MCP specification over stdio (JSON-RPC 2.0).
//! Connects to a running EvaporChain node via HTTP API.
//!
//! 26 Tools · 13 Resources · 6 Prompts
//!
//! Start: evaporchain-mcp [--node-url http://NODE:PORT]
//!
//! ## Security configuration (CRITICAL-2 partial fix, 2026-05-06)
//!
//! Per `AUDIT_2026_05_06.md` CRITICAL-2 — this MCP server is the
//! AI-agent attack surface. Three env vars control authentication:
//!
//!   `EVAPORCHAIN_MCP_API_TOKEN` (optional)
//!     If set, the value is sent as `Authorization: Bearer <token>`
//!     on every outgoing HTTP request to the node. Backend
//!     enforcement is currently advisory (the node API does not yet
//!     verify the header — that work is queued as a follow-up).
//!
//!   `EVAPORCHAIN_MCP_REQUIRE_AUTH` (default: "false")
//!     If "true" / "1" / "yes", the binary refuses to start unless
//!     `EVAPORCHAIN_MCP_API_TOKEN` is also set. Use in production
//!     deployments where zero-auth-by-default is unacceptable.
//!
//!   `EVAPORCHAIN_MCP_NODE_URL` (default: http://127.0.0.1:8080)
//!     Default node URL when `--node-url` flag is not supplied.
//!     Replaces the previous hardcoded `http://37.27.1.1:8080`.

mod prompts;
mod protocol;
mod resources;
mod tools;

use std::env;
use tokio::io::{self, AsyncBufReadExt, BufReader};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ── Node URL: --node-url flag overrides env var, env var overrides default ──
    let node_url = env::args()
        .skip_while(|a| a != "--node-url")
        .nth(1)
        .or_else(|| env::var("EVAPORCHAIN_MCP_NODE_URL").ok())
        .unwrap_or_else(|| "http://127.0.0.1:8080".to_string());

    // ── Auth token + require-auth gate (CRITICAL-2 hardening) ──
    let api_token = env::var("EVAPORCHAIN_MCP_API_TOKEN")
        .ok()
        .filter(|t| !t.is_empty());
    let require_auth = matches!(
        env::var("EVAPORCHAIN_MCP_REQUIRE_AUTH").as_deref(),
        Ok("true") | Ok("1") | Ok("TRUE") | Ok("yes")
    );

    if require_auth && api_token.is_none() {
        eprintln!(
            "evaporchain-mcp: refusing to start.\n\
             EVAPORCHAIN_MCP_REQUIRE_AUTH is set but \
             EVAPORCHAIN_MCP_API_TOKEN is not. Either set the token \
             or unset EVAPORCHAIN_MCP_REQUIRE_AUTH for dev mode."
        );
        std::process::exit(2);
    }

    eprintln!(
        "evaporchain-mcp: starting (node_url={}, auth={})",
        node_url,
        if api_token.is_some() {
            "Bearer-token"
        } else {
            "none-DEV-MODE"
        }
    );

    let client = reqwest::Client::new();
    let ctx = protocol::Context {
        node_url,
        client,
        api_token,
    };

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
