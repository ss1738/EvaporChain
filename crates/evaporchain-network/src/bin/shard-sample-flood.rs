//! `shard-sample-flood` — T0.7 Vector 6 operator-acceptance harness.
//!
//! Connects to a target validator as a libp2p peer and floods it with
//! `ShardSampleRequest` payloads to exercise the two defenses landed in
//! commit `8c59fad` (AUDIT-2026-05-11-1/2):
//!
//!   1. **Per-peer rate-limit** — gossipsub/sync per-peer budget now
//!      gates ShardSample inbound too. A peer that has saturated its
//!      budget should be rejected with `Rate-limited peer …` warn.
//!   2. **Queries cap** — `MAX_SHARD_QUERIES_PER_REQUEST = 256`. Each
//!      overshoot drops the request AND records a peer violation
//!      (warn: `Peer … sent N shard queries, cap is 256 — recording
//!      violation`).
//!
//! Why a Rust binary rather than `scripts/dos-flood.sh`: ShardSample
//! is a libp2p `request_response::json::Behaviour` protocol, not HTTP.
//! Bash + curl can't speak the binary wire format. This binary uses
//! the existing `P2pNetworkService` machinery so the harness shares
//! the production codec.
//!
//! ## Usage
//!
//! ```bash
//! cargo run --release --bin shard-sample-flood -- \
//!     --target-addr /ip4/100.119.53.101/tcp/9000/p2p/<peer-id> \
//!     --chain-id evaporchain-tailscale-5node-1 \
//!     --rate 1000 \
//!     --queries-per-request 1024 \
//!     --duration 60s
//! ```
//!
//! ## Pass criteria (operator-side, on the TARGET node)
//!
//! Watch the target's stderr / journal output. With the defenses in
//! place:
//!
//! - Most of the flooded requests rejected with `Rate-limited peer
//!   {peer} — dropping shard-sample request`
//! - Every request with `queries.len() > 256` rejected with `Peer
//!   {peer} sent {N} shard queries, cap is 256 — recording violation`
//! - After enough violations, the peer ends up in the ban list
//!   (governed by `PeerBanList::record_violation` thresholds)
//! - Target node's CPU does NOT spike for ShardSample serving
//!
//! Without the defenses, the target would burn a Merkle-proof
//! construction per query — `queries.len() = 1024` × `rate = 1000/s`
//! = ~1M proofs/sec demand, which would saturate a single core.
//!
//! See `docs/runbooks/dos-resistance.md` Vector 6.

use std::time::{Duration, Instant};

use evaporchain_da::sampling::SampleQuery;
use evaporchain_network::{NetworkConfig, P2pNetworkService};
use tracing::{info, warn};

const VERSION: &str = "1.0.0";

struct Args {
    target_addr: String,
    chain_id: String,
    rate_per_sec: u64,
    queries_per_request: usize,
    duration: Duration,
}

fn parse_duration(s: &str) -> Result<Duration, String> {
    let s = s.trim();
    let (num, unit): (&str, u64) = if let Some(n) = s.strip_suffix('h') {
        (n, 3600)
    } else if let Some(n) = s.strip_suffix('m') {
        (n, 60)
    } else if let Some(n) = s.strip_suffix('s') {
        (n, 1)
    } else {
        (s, 1)
    };
    let secs: u64 = num
        .parse()
        .map_err(|_| format!("invalid duration: '{s}'"))?;
    Ok(Duration::from_secs(secs * unit))
}

fn parse_args() -> Result<Args, String> {
    let mut target_addr: Option<String> = None;
    let mut chain_id = String::new();
    let mut rate_per_sec: u64 = 100;
    let mut queries_per_request: usize = 1024;
    let mut duration = Duration::from_secs(60);

    let raw: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < raw.len() {
        match raw[i].as_str() {
            "--target-addr" => {
                target_addr = Some(raw.get(i + 1).cloned().ok_or("--target-addr needs a value")?);
                i += 2;
            }
            "--chain-id" => {
                chain_id = raw.get(i + 1).cloned().ok_or("--chain-id needs a value")?;
                i += 2;
            }
            "--rate" => {
                rate_per_sec = raw
                    .get(i + 1)
                    .ok_or("--rate needs a value")?
                    .parse()
                    .map_err(|_| "--rate must be an integer")?;
                i += 2;
            }
            "--queries-per-request" => {
                queries_per_request = raw
                    .get(i + 1)
                    .ok_or("--queries-per-request needs a value")?
                    .parse()
                    .map_err(|_| "--queries-per-request must be an integer")?;
                i += 2;
            }
            "--duration" => {
                duration = parse_duration(raw.get(i + 1).ok_or("--duration needs a value")?)?;
                i += 2;
            }
            "-h" | "--help" => {
                print_usage();
                std::process::exit(0);
            }
            "--version" => {
                println!("shard-sample-flood {VERSION}");
                std::process::exit(0);
            }
            other => return Err(format!("unknown arg: {other}")),
        }
    }
    let target_addr = target_addr.ok_or_else(|| "--target-addr required".to_string())?;
    if rate_per_sec == 0 {
        return Err("--rate must be > 0".to_string());
    }
    Ok(Args {
        target_addr,
        chain_id,
        rate_per_sec,
        queries_per_request,
        duration,
    })
}

fn print_usage() {
    println!("shard-sample-flood {VERSION}");
    println!();
    println!("T0.7 Vector 6 operator-acceptance harness — floods a target node");
    println!("with libp2p ShardSampleRequest payloads to exercise the rate-limit");
    println!("and queries-cap defenses landed in commit 8c59fad.");
    println!();
    println!("Usage:");
    println!("    shard-sample-flood --target-addr <multiaddr> [options]");
    println!();
    println!("Options:");
    println!("    --target-addr <multiaddr>     Required. Bootstrap multiaddr.");
    println!("                                  Format: /ip4/HOST/tcp/PORT/p2p/PEER_ID");
    println!("    --chain-id <id>               Chain id (must match target). Default: empty.");
    println!("    --rate <N>                    Requests per second. Default: 100.");
    println!("    --queries-per-request <N>     Queries per request. Default: 1024.");
    println!("                                  (>256 triggers the cap-violation gate.)");
    println!("    --duration <Ns|Nm|Nh>         Run duration. Default: 60s.");
    println!("    -h, --help                    This help text.");
    println!("    --version                     Print version and exit.");
    println!();
    println!("Pass criteria — watch the TARGET node's stderr:");
    println!("    'Rate-limited peer ... — dropping shard-sample request'");
    println!("    'Peer ... sent N shard queries, cap is 256 — recording violation'");
    println!();
    println!("See docs/runbooks/dos-resistance.md Vector 6.");
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            eprintln!("run with --help for usage");
            std::process::exit(2);
        }
    };

    info!(
        target = %args.target_addr,
        chain_id = %args.chain_id,
        rate = args.rate_per_sec,
        queries = args.queries_per_request,
        duration_s = args.duration.as_secs(),
        "shard-sample-flood starting"
    );
    if args.queries_per_request > 256 {
        warn!(
            "queries_per_request = {} (> MAX_SHARD_QUERIES_PER_REQUEST = 256) — \
             each request will trigger the cap-violation gate on the target",
            args.queries_per_request
        );
    }

    let cfg = NetworkConfig {
        listen_address: "/ip4/0.0.0.0/tcp/0".to_string(),
        bootstrap_peers: vec![args.target_addr.clone()],
        chain_id: args.chain_id.clone(),
        ..NetworkConfig::default()
    };

    let (ch, _handle, my_peer_id) = P2pNetworkService::start(cfg).await?;
    info!(my_peer_id = %my_peer_id, "harness started; waiting 3s for dial completion");
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Build the oversized query batch once. block_number = 1 is a
    // reasonable default — the target's response cache may not have it,
    // but the rate-limit / cap gates fire BEFORE the cache lookup, so
    // cache miss is irrelevant for this harness.
    let queries: Vec<SampleQuery> = (0..args.queries_per_request)
        .map(|i| SampleQuery {
            block_number: 1,
            shard_index: i % 32,
        })
        .collect();

    let start = Instant::now();
    let sleep_us = 1_000_000u64.checked_div(args.rate_per_sec).unwrap_or(1);
    let sleep_per = Duration::from_micros(sleep_us);
    let mut sent: u64 = 0;
    let mut send_errors: u64 = 0;

    while start.elapsed() < args.duration {
        match ch.sample_request_sender.send(queries.clone()).await {
            Ok(_) => sent += 1,
            Err(e) => {
                send_errors += 1;
                if send_errors <= 5 {
                    warn!("sample_request_sender error: {e}");
                }
                if send_errors > 100 {
                    return Err("sample_request_sender saturated — service likely dead".into());
                }
            }
        }
        if sent.is_multiple_of(1000) && sent > 0 {
            let elapsed = start.elapsed().as_secs_f64();
            info!(
                sent,
                send_errors,
                observed_rate = sent as f64 / elapsed,
                "flood progress"
            );
        }
        if sleep_us > 0 {
            tokio::time::sleep(sleep_per).await;
        }
    }

    let elapsed = start.elapsed().as_secs_f64();
    info!(
        sent,
        send_errors,
        elapsed_s = elapsed,
        observed_rate = sent as f64 / elapsed,
        "flood complete"
    );
    println!();
    println!("--- flood summary ---");
    println!("  sent:           {sent}");
    println!("  send errors:    {send_errors}");
    println!("  elapsed:        {elapsed:.1}s");
    println!("  observed rate:  {:.1} req/s", sent as f64 / elapsed);
    println!();
    println!("Now inspect the TARGET node's logs for:");
    println!("  - 'Rate-limited peer ... — dropping shard-sample request'");
    println!("  - 'Peer ... sent N shard queries, cap is 256 — recording violation'");
    println!("See docs/runbooks/dos-resistance.md Vector 6 for full acceptance criteria.");

    Ok(())
}
