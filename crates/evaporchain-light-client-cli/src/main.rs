//! Operator-facing CLI for the Light Client SDK.
//!
//! Three subcommands:
//!
//!   sync-latest --node URL [--genesis-height N]
//!     Fetch latest header from the node, walk forward from the
//!     genesis-height anchor (default: latest - 0 = no walk if not
//!     supplied — caller must seed an initial trust anchor),
//!     verify each block via the BFT BLS aggregate-sig path, and
//!     print the new trusted tip as JSON.
//!
//!   get-state --node URL --key HEX [--expected HEX]
//!     Fetch a Verkle state-query proof for the given 32-byte
//!     trie key, verify it cryptographically against the trusted
//!     state_root from the latest synced header, and print the
//!     proof's value (or `null` for non-membership). If
//!     --expected is supplied, additionally assert the value
//!     matches it.
//!
//!   watch --node URL [--genesis-height N] [--poll-secs N]
//!     Follow the chain forward indefinitely. Polls /api/light_
//!     header/latest every N seconds (default 5), runs sync, and
//!     prints `{height, state_root, ingested}` per cycle. Cancel
//!     with Ctrl-C.
//!
//! Useful as both an operator tool (quickly probe a node) and a
//! worked-example reference for downstream consumers (mobile /
//! browser / bridge / explorer).

use clap::{Parser, Subcommand};
use evaporchain_light_client::transport::RpcTransport;
use evaporchain_light_client::LightClient;
use evaporchain_light_client_http::HttpTransport;
use std::process::ExitCode;
use std::time::SystemTime;

/// EvaporChain Light Client CLI.
#[derive(Parser, Debug)]
#[command(
    name = "evaporchain-light-client",
    about = "Operator CLI for the EvaporChain Light Client SDK.",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Fetch latest header, walk forward from genesis-height,
    /// print the new trusted tip as JSON.
    SyncLatest {
        /// Node base URL (e.g., "http://localhost:8080").
        #[arg(long)]
        node: String,
        /// Optional initial trust anchor height. If unset, the
        /// CLI seeds the trust anchor at the latest header itself
        /// (zero-walk; useful when the operator already trusts
        /// the node's claim of "latest").
        #[arg(long)]
        genesis_height: Option<u64>,
        /// Optional bearer token for nodes behind auth gateways.
        #[arg(long)]
        bearer_token: Option<String>,
    },

    /// Fetch + verify a state-query proof for the given trie key
    /// (or account address — see --account).
    GetState {
        /// Node base URL.
        #[arg(long)]
        node: String,
        /// 64-character hex 32-byte trie key. Mutually exclusive
        /// with --account; one of the two is required.
        #[arg(long, conflicts_with = "account")]
        key: Option<String>,
        /// 64-character hex 32-byte account address. The CLI
        /// derives the trie key as
        /// `blake3("acct" || address)` — same formula
        /// `evaporchain_state::trie_key_for_account` uses.
        /// Mutually exclusive with --key; one of the two is
        /// required.
        #[arg(long, conflicts_with = "key")]
        account: Option<String>,
        /// Optional 64-character hex 32-byte expected value. If
        /// supplied, the CLI fails non-zero on mismatch.
        #[arg(long)]
        expected: Option<String>,
        /// Optional bearer token.
        #[arg(long)]
        bearer_token: Option<String>,
    },

    /// Follow the chain forward indefinitely.
    Watch {
        /// Node base URL.
        #[arg(long)]
        node: String,
        /// Initial trust anchor height. Required for a meaningful
        /// watch — without it, every poll seeds at the latest and
        /// reports zero-walk.
        #[arg(long)]
        genesis_height: u64,
        /// Polling cadence in seconds. Default 5.
        #[arg(long, default_value_t = 5)]
        poll_secs: u64,
        /// Optional bearer token.
        #[arg(long)]
        bearer_token: Option<String>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Cmd::SyncLatest {
            node,
            genesis_height,
            bearer_token,
        } => run_sync_latest(node, genesis_height, bearer_token),
        Cmd::GetState {
            node,
            key,
            account,
            expected,
            bearer_token,
        } => run_get_state(node, key, account, expected, bearer_token),
        Cmd::Watch {
            node,
            genesis_height,
            poll_secs,
            bearer_token,
        } => run_watch(node, genesis_height, poll_secs, bearer_token),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("error: {msg}");
            ExitCode::FAILURE
        }
    }
}

fn current_time_secs() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn build_transport(node: &str, bearer_token: Option<String>) -> HttpTransport {
    let mut transport = HttpTransport::new(node);
    if let Some(t) = bearer_token {
        transport = transport.with_bearer_token(t);
    }
    transport
}

fn run_sync_latest(
    node: String,
    genesis_height: Option<u64>,
    bearer_token: Option<String>,
) -> Result<(), String> {
    let transport = build_transport(&node, bearer_token);
    let now = current_time_secs();

    // Fetch the genesis (or latest) anchor.
    let genesis = match genesis_height {
        Some(h) => transport
            .fetch_header_at(h)
            .map_err(|e| format!("fetch genesis at height {h}: {e}"))?,
        None => transport
            .fetch_latest_header()
            .map_err(|e| format!("fetch latest as genesis: {e}"))?,
    };

    let mut lc = LightClient::new(genesis.clone(), now, /* vk_bytes */ None);

    // Walk forward to the chain's reported latest.
    let latest = transport
        .fetch_latest_header()
        .map_err(|e| format!("fetch latest: {e}"))?;
    let target = latest.height;
    let _ingested = lc
        .sync_to_height(&transport, target, now)
        .map_err(|e| format!("sync_to_height({target}): {e}"))?;

    let out = serde_json::json!({
        "trusted_tip_height": lc.current_height(),
        "trusted_tip_state_root": hex_lower(&lc.current_state_root()),
        "genesis_anchor_height": genesis.height,
        "trust_period_secs": lc.trust_period_secs(),
    });
    println!("{}", serde_json::to_string_pretty(&out).unwrap());
    Ok(())
}

fn run_get_state(
    node: String,
    key_hex: Option<String>,
    account_hex: Option<String>,
    expected_hex: Option<String>,
    bearer_token: Option<String>,
) -> Result<(), String> {
    let transport = build_transport(&node, bearer_token);
    let now = current_time_secs();

    // Resolve the trie key from either --key or --account. clap's
    // `conflicts_with` ensures we never get both; we still need to
    // require one.
    let (key, source_label) = match (key_hex.as_deref(), account_hex.as_deref()) {
        (Some(k), None) => {
            let k_arr = parse_hex_32(k)
                .ok_or_else(|| format!("--key must be 64-char hex: {k}"))?;
            (k_arr, format!("key={k}"))
        }
        (None, Some(a)) => {
            let addr = parse_hex_32(a)
                .ok_or_else(|| format!("--account must be 64-char hex: {a}"))?;
            // Same formula as evaporchain-state::db::trie_key_for_account:
            //   blake3("acct" || address)
            let mut buf = Vec::with_capacity(36);
            buf.extend_from_slice(b"acct");
            buf.extend_from_slice(&addr);
            let derived = *blake3::hash(&buf).as_bytes();
            (derived, format!("account={a}"))
        }
        (None, None) => {
            return Err(
                "one of --key (raw 32-byte trie key) or --account (32-byte address)
 is required"
                    .to_string(),
            );
        }
        (Some(_), Some(_)) => unreachable!("clap conflicts_with prevents both"),
    };

    let expected = match expected_hex.as_deref() {
        Some(s) => Some(
            parse_hex_32(s).ok_or_else(|| format!("--expected must be 64-char hex: {s}"))?,
        ),
        None => None,
    };

    // Anchor the LightClient at the chain's latest header.
    let latest = transport
        .fetch_latest_header()
        .map_err(|e| format!("fetch latest as anchor: {e}"))?;
    let lc = LightClient::new(latest.clone(), now, /* vk_bytes */ None);

    let value = lc
        .fetch_and_verify_state(&transport, &key, expected)
        .map_err(|e| format!("verify state at {source_label}: {e}"))?;

    let out = serde_json::json!({
        "trusted_tip_height": lc.current_height(),
        "trusted_tip_state_root": hex_lower(&lc.current_state_root()),
        "queried": source_label,
        "trie_key": hex_lower(&key),
        "value": value.map(|v| hex_lower(&v)),
    });
    println!("{}", serde_json::to_string_pretty(&out).unwrap());
    Ok(())
}

fn run_watch(
    node: String,
    genesis_height: u64,
    poll_secs: u64,
    bearer_token: Option<String>,
) -> Result<(), String> {
    let transport = build_transport(&node, bearer_token);
    let now = current_time_secs();

    let genesis = transport
        .fetch_header_at(genesis_height)
        .map_err(|e| format!("fetch genesis at {genesis_height}: {e}"))?;
    let mut lc = LightClient::new(genesis, now, /* vk_bytes */ None);

    eprintln!(
        "watching node {node} from height {genesis_height} (poll every {poll_secs}s; Ctrl-C to stop)"
    );

    loop {
        let cycle_now = current_time_secs();
        let prev_height = lc.current_height();
        match lc.sync_to_latest(&transport, cycle_now) {
            Ok(new_height) => {
                let ingested = new_height.saturating_sub(prev_height);
                let out = serde_json::json!({
                    "height": new_height,
                    "state_root": hex_lower(&lc.current_state_root()),
                    "ingested_this_cycle": ingested,
                });
                println!("{}", serde_json::to_string(&out).unwrap());
            }
            Err(e) => {
                eprintln!("warn: sync cycle failed (preserving last trusted tip {prev_height}): {e}");
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(poll_secs));
    }
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
    fn parse_hex_32_ok() {
        let s = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let parsed = parse_hex_32(s).unwrap();
        assert_eq!(parsed[0], 0x01);
        assert_eq!(parsed[31], 0xef);
    }

    #[test]
    fn parse_hex_32_rejects_short() {
        assert!(parse_hex_32("abc").is_none());
    }

    #[test]
    fn parse_hex_32_rejects_non_hex() {
        let s = "zz23456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        assert!(parse_hex_32(s).is_none());
    }

    #[test]
    fn hex_lower_round_trip() {
        let bytes: [u8; 4] = [0x00, 0x12, 0xab, 0xff];
        assert_eq!(hex_lower(&bytes), "0012abff");
    }

    #[test]
    fn cli_parses_sync_latest_with_node() {
        let cli = Cli::try_parse_from(["lc", "sync-latest", "--node", "http://localhost:8080"])
            .expect("must parse");
        match cli.command {
            Cmd::SyncLatest { node, .. } => assert_eq!(node, "http://localhost:8080"),
            _ => panic!("wrong subcommand"),
        }
    }

    #[test]
    fn cli_parses_get_state_with_key() {
        let key = "0".repeat(64);
        let cli = Cli::try_parse_from([
            "lc",
            "get-state",
            "--node",
            "http://localhost:8080",
            "--key",
            &key,
        ])
        .expect("must parse");
        match cli.command {
            Cmd::GetState { node, key: k, account, .. } => {
                assert_eq!(node, "http://localhost:8080");
                assert_eq!(k, Some(key));
                assert_eq!(account, None);
            }
            _ => panic!("wrong subcommand"),
        }
    }

    #[test]
    fn cli_parses_get_state_with_account() {
        let addr = "0".repeat(64);
        let cli = Cli::try_parse_from([
            "lc",
            "get-state",
            "--node",
            "http://localhost:8080",
            "--account",
            &addr,
        ])
        .expect("must parse");
        match cli.command {
            Cmd::GetState { account, key, .. } => {
                assert_eq!(account, Some(addr));
                assert_eq!(key, None);
            }
            _ => panic!("wrong subcommand"),
        }
    }

    #[test]
    fn cli_get_state_rejects_both_key_and_account() {
        let result = Cli::try_parse_from([
            "lc",
            "get-state",
            "--node",
            "http://localhost:8080",
            "--key",
            &"0".repeat(64),
            "--account",
            &"1".repeat(64),
        ]);
        assert!(result.is_err(), "clap should reject both --key and --account together");
    }

    #[test]
    fn cli_watch_requires_genesis_height() {
        // Without --genesis-height, parse should fail.
        let result = Cli::try_parse_from(["lc", "watch", "--node", "http://localhost:8080"]);
        assert!(result.is_err());
    }
}
