//! Coordinator runtime configuration.
//!
//! All values default to the mock-consensus devnet. Override via
//! environment variables or CLI flags in main.

/// How often the coordinator polls `/api/scripts` for new listings.
pub const POLL_INTERVAL_MS: u64 = 2_000;

/// How long the coordinator waits for a submitted TX to finalise
/// before timing out and logging a warning.
pub const FINALITY_TIMEOUT_MS: u64 = 30_000;

/// How often the coordinator re-polls tx status during finality wait.
pub const FINALITY_POLL_MS: u64 = 500;

#[derive(Debug, Clone)]
pub struct Config {
    /// Base URL of the running EvaporChain node API (no trailing slash).
    pub node_url: String,
    /// Bearer token for authenticated endpoints (`/api/tx/*`).
    /// Set to the empty string if the node has auth disabled.
    pub auth_token: String,
    /// `caller` byte used when submitting `record_sale`. The genesis
    /// all-zeros account (byte=0) works in mock-consensus devnet.
    pub caller_byte: u8,
    /// Port for the coordinator's own HTTP bid-intake server.
    pub bid_server_port: u16,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            node_url: "http://localhost:8099".into(),
            auth_token: String::new(),
            caller_byte: 0,
            bid_server_port: 9100,
        }
    }
}

impl Config {
    /// Construct from environment variables. Falls back to defaults.
    pub fn from_env() -> Self {
        Self {
            node_url: std::env::var("SFSV_NODE_URL")
                .unwrap_or_else(|_| "http://localhost:8099".into()),
            auth_token: std::env::var("SFSV_AUTH_TOKEN").unwrap_or_default(),
            caller_byte: std::env::var("SFSV_CALLER_BYTE")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
            bid_server_port: std::env::var("SFSV_BID_PORT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(9100),
        }
    }
}
