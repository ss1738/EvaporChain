//! Worked-example: balance-monitor.
//!
//! Polls a single account's state-query proof from a node at a
//! fixed cadence; prints a JSON line every time the verified value
//! changes (or on first observation). Demonstrates the Light Client
//! SDK end-to-end:
//!
//!   * Anchor a `LightClient` at the chain's reported latest header
//!     (BFT-verified by the SDK at construction).
//!   * Walk the trust anchor forward each cycle via
//!     `LightClient::sync_to_latest`.
//!   * Verify a state-query proof at the new tip via
//!     `LightClient::fetch_and_verify_state`.
//!
//! Use this as a copyable starting point for a wallet/dapp/explorer
//! integration. Real consumers will want to:
//!
//!   * Persist the trusted tip across restarts (this example holds
//!     the LightClient in memory only — restart re-anchors at chain
//!     latest, defeating the trust period's purpose).
//!   * Handle long-running trust-period expiry by re-anchoring at a
//!     fresh checkpoint.
//!   * Plug in their own UI / notification path instead of stdout.
//!
//! Cluster-safe: this binary is read-only against the node.

use clap::Parser;
use evaporchain_light_client::transport::RpcTransport;
use evaporchain_light_client::LightClient;
use evaporchain_light_client_http::HttpTransport;
use std::process::ExitCode;
use std::time::SystemTime;

#[derive(Parser, Debug)]
#[command(
    name = "evaporchain-balance-monitor",
    about = "Light Client SDK worked example: monitor one account's verified state.",
    version
)]
struct Cli {
    /// Node base URL, e.g. "http://localhost:8081". No trailing slash.
    #[arg(long)]
    node: String,

    /// 64-char hex 32-byte account address. Trie key is derived as
    /// `blake3("acct" || addr)` (matches the chain's
    /// `evaporchain_state::db::trie_key_for_account`).
    #[arg(long)]
    account: String,

    /// Polling cadence in seconds. Default 10.
    #[arg(long, default_value_t = 10)]
    poll_secs: u64,

    /// Optional initial trust anchor height. If unset, anchors at
    /// chain latest (zero-walk on startup).
    #[arg(long)]
    genesis_height: Option<u64>,

    /// Optional bearer token sent as `Authorization: Bearer <T>`.
    #[arg(long)]
    bearer_token: Option<String>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("error: {msg}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), String> {
    // Step 1: parse + derive the trie key once.
    let addr = parse_hex_32(&cli.account)
        .ok_or_else(|| format!("--account must be 64-char hex: {}", cli.account))?;
    let trie_key = derive_account_trie_key(&addr);

    // Step 2: build the transport.
    let mut transport = HttpTransport::new(&cli.node);
    if let Some(t) = cli.bearer_token {
        transport = transport.with_bearer_token(t);
    }

    // Step 3: anchor the LightClient.
    let now = current_time_secs();
    let genesis = match cli.genesis_height {
        Some(h) => transport
            .fetch_header_at(h)
            .map_err(|e| format!("fetch genesis at {h}: {e}"))?,
        None => transport
            .fetch_latest_header()
            .map_err(|e| format!("fetch latest as anchor: {e}"))?,
    };
    let mut lc = LightClient::new(genesis, now, /* vk_bytes */ None);

    eprintln!(
        "monitoring account {} on {} (poll every {}s; Ctrl-C to stop)",
        cli.account, cli.node, cli.poll_secs
    );
    eprintln!(
        "anchored at height {} state_root {}",
        lc.current_height(),
        hex_lower(&lc.current_state_root())
    );

    // Step 4: poll loop. Keeps the last-observed value to suppress
    // repeated unchanged events.
    let mut last_value: Option<Option<[u8; 32]>> = None;
    loop {
        let cycle_now = current_time_secs();

        // Walk the trust anchor forward.
        let prev_height = lc.current_height();
        match lc.sync_to_latest(&transport, cycle_now) {
            Ok(_new_height) => {
                let _ingested = lc.current_height().saturating_sub(prev_height);
            }
            Err(e) => {
                eprintln!(
                    "warn: sync_to_latest failed (preserving last trusted tip {prev_height}): {e}"
                );
                std::thread::sleep(std::time::Duration::from_secs(cli.poll_secs));
                continue;
            }
        }

        // Verify the account's state at the new tip.
        match lc.fetch_and_verify_state(&transport, &trie_key, /* expected */ None) {
            Ok(value) => {
                if last_value.as_ref() != Some(&value) {
                    let out = serde_json::json!({
                        "height": lc.current_height(),
                        "state_root": hex_lower(&lc.current_state_root()),
                        "account": cli.account,
                        "trie_key": hex_lower(&trie_key),
                        "value": value.map(|v| hex_lower(&v)),
                    });
                    println!("{}", serde_json::to_string(&out).unwrap());
                    last_value = Some(value);
                }
            }
            Err(e) => {
                eprintln!("warn: state-proof verify failed at height {}: {e}", lc.current_height());
            }
        }

        std::thread::sleep(std::time::Duration::from_secs(cli.poll_secs));
    }
}

fn current_time_secs() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn derive_account_trie_key(addr: &[u8; 32]) -> [u8; 32] {
    let mut buf = Vec::with_capacity(36);
    buf.extend_from_slice(b"acct");
    buf.extend_from_slice(addr);
    *blake3::hash(&buf).as_bytes()
}

fn parse_hex_32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let bytes = s.as_bytes();
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        let hi = hex_nibble(bytes[i * 2])?;
        let lo = hex_nibble(bytes[i * 2 + 1])?;
        *byte = (hi << 4) | lo;
    }
    Some(out)
}

fn hex_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(nibble_hex(b >> 4));
        s.push(nibble_hex(b & 0x0f));
    }
    s
}

fn nibble_hex(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        10..=15 => (b'a' + (n - 10)) as char,
        _ => '?',
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_trie_key_matches_blake3_acct_addr() {
        let addr = [0x04u8; 32];
        let derived = derive_account_trie_key(&addr);
        let mut expected_input = Vec::with_capacity(36);
        expected_input.extend_from_slice(b"acct");
        expected_input.extend_from_slice(&addr);
        let expected = *blake3::hash(&expected_input).as_bytes();
        assert_eq!(derived, expected);
    }

    #[test]
    fn parse_hex_32_round_trip() {
        let s = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let parsed = parse_hex_32(s).unwrap();
        assert_eq!(hex_lower(&parsed), s);
    }

    #[test]
    fn cli_parses_minimum_args() {
        let cli = Cli::try_parse_from([
            "balance-monitor",
            "--node",
            "http://localhost:8081",
            "--account",
            &"0".repeat(64),
        ])
        .expect("must parse");
        assert_eq!(cli.node, "http://localhost:8081");
        assert_eq!(cli.poll_secs, 10);
        assert!(cli.genesis_height.is_none());
    }

    #[test]
    fn cli_accepts_full_args() {
        let cli = Cli::try_parse_from([
            "balance-monitor",
            "--node",
            "http://localhost:8081",
            "--account",
            &"0".repeat(64),
            "--poll-secs",
            "3",
            "--genesis-height",
            "15190",
            "--bearer-token",
            "tok",
        ])
        .expect("must parse");
        assert_eq!(cli.poll_secs, 3);
        assert_eq!(cli.genesis_height, Some(15190));
        assert_eq!(cli.bearer_token.as_deref(), Some("tok"));
    }
}
