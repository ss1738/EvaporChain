//! evaporchain-faucet — public testnet faucet HTTP service.
//!
//! Sits in front of an evaporchain-node and exposes a small public HTTP
//! interface for testnet token claims. Per-IP × per-address rate limit,
//! optional Cloudflare Turnstile / hCaptcha gate, persistent claim
//! history in a JSON file under a tokio mutex.
//!
//! Endpoints:
//!   GET  /         — static HTML form page
//!   POST /claim    — body {address, captcha_response?} → forwards to
//!                    <node-url>/api/faucet with the configured admin token.
//!   GET  /health   — {ok, claims_total, claims_today, rate_limit_per_12h}
//!
//! Hard constraints:
//!   - No new heavy deps. Storage is JSON + tokio::sync::Mutex.
//!   - Admin token to the chain's /api/faucet endpoint is supplied via
//!     --admin-token (or empty if the node isn't running with
//!     EVAPORCHAIN_ADMIN_KEY).

use anyhow::{Context, Result};
use axum::{
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

// ─────────────────────────── CLI ──────────────────────────────────────────

#[derive(clap::Parser, Clone)]
#[command(
    name = "evaporchain-faucet",
    about = "Public testnet faucet for EvaporChain."
)]
struct Cli {
    /// Listen port for the public HTTP service.
    #[arg(long, default_value_t = 7676)]
    port: u16,

    /// URL of the running evaporchain-node (its /api/faucet endpoint).
    #[arg(long, default_value = "http://localhost:8080")]
    node_url: String,

    /// Bearer token for the node's admin endpoints. If the node is
    /// running with EVAPORCHAIN_ADMIN_KEY=… this MUST match.
    #[arg(long, default_value = "")]
    admin_token: String,

    /// Max claims per (ip, address) within the rate-limit window.
    #[arg(long, default_value_t = 1)]
    rate_limit_per_12h: u32,

    /// Path to a JSON file storing per-IP claim history. Created on first
    /// use. Survives restarts.
    #[arg(long, default_value = "./faucet-claims.json")]
    rate_limit_storage: PathBuf,

    /// Optional CAPTCHA secret. If set, /claim requires
    /// `captcha_response` and validates it against either Cloudflare
    /// Turnstile or hCaptcha (auto-detected from prefix). If unset, the
    /// faucet is naked (testnet default).
    #[arg(long)]
    captcha_key: Option<String>,

    /// Maximum tokens (in EVAP) any one claim is allowed to dispense.
    /// Surfaced in /health and in the front page form copy; the actual
    /// per-claim amount is determined by the chain's faucet handler, so
    /// this is informational + hard cap.
    #[arg(long, default_value_t = 1000)]
    max_claim_amount: u64,
}

// ─────────────────────────── Persistent rate-limit store ─────────────────

/// On-disk shape: keyed by `ip|address_lowercase` → list of unix-seconds
/// timestamps. Trimmed to the rate-limit window on every read.
#[derive(Default, Serialize, Deserialize)]
struct ClaimStore {
    entries: HashMap<String, Vec<u64>>,
    /// Cumulative claim counter (success only). Useful for /health.
    total: u64,
}

impl ClaimStore {
    fn load(path: &PathBuf) -> Self {
        match std::fs::read_to_string(path) {
            Ok(s) if !s.trim().is_empty() => serde_json::from_str(&s).unwrap_or_default(),
            _ => Self::default(),
        }
    }

    fn save(&self, path: &PathBuf) -> Result<()> {
        let json = serde_json::to_string_pretty(self).context("encode claim store")?;
        // Audit B1 (2026-05-14): atomic write — tmp + fsync + rename.
        // Without fsync the OS may not flush pages before the rename,
        // and a crash can leave the claim-store silently cleared.
        let tmp = path.with_extension("json.tmp");
        {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp)
                .with_context(|| format!("create {}", tmp.display()))?;
            f.write_all(json.as_bytes())
                .with_context(|| format!("write {}", tmp.display()))?;
            f.sync_all()
                .with_context(|| format!("fsync {}", tmp.display()))?;
        }
        std::fs::rename(&tmp, path).with_context(|| format!("rename to {}", path.display()))?;
        Ok(())
    }
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

const WINDOW_SECS: u64 = 12 * 3600;

fn key(ip: &IpAddr, addr_norm: &str) -> String {
    format!("{}|{}", ip, addr_norm)
}

/// Returns Ok(()) if the claim is allowed under the limit. Mutates the
/// store: on Ok, the new timestamp is appended; on Err, the store is
/// untouched. Caller is responsible for persisting via `save`.
fn check_and_record(
    store: &mut ClaimStore,
    ip: &IpAddr,
    address: &str,
    limit: u32,
    now: u64,
) -> Result<(), String> {
    let k = key(ip, address);
    let entry = store.entries.entry(k).or_default();
    // Trim entries outside the 12h window.
    entry.retain(|t| now.saturating_sub(*t) < WINDOW_SECS);
    if (entry.len() as u32) >= limit {
        let oldest = *entry.iter().min().unwrap_or(&now);
        let retry_in = WINDOW_SECS.saturating_sub(now.saturating_sub(oldest));
        return Err(format!(
            "rate limited: {} claim(s) per {}h per IP+address. Try again in {}m.",
            limit,
            WINDOW_SECS / 3600,
            retry_in / 60 + 1
        ));
    }
    entry.push(now);
    store.total += 1;
    Ok(())
}

/// claims_today = entries with timestamp ≥ midnight UTC today.
fn claims_today(store: &ClaimStore, now: u64) -> u64 {
    let today_midnight = now - (now % 86_400);
    let mut n = 0u64;
    for ts_list in store.entries.values() {
        for &t in ts_list {
            if t >= today_midnight {
                n += 1;
            }
        }
    }
    n
}

// ─────────────────────────── CAPTCHA verification ────────────────────────

#[derive(Deserialize)]
struct CaptchaSiteVerifyResponse {
    success: bool,
}

/// Validates a captcha response against either Turnstile or hCaptcha.
/// Auto-detects via the `captcha_response` token prefix:
///   - Cloudflare Turnstile tokens are typed via the secret prefix
///     `0x` (Turnstile) — but in practice we just try the Turnstile
///     endpoint first, fall back to hCaptcha.
async fn verify_captcha(
    client: &reqwest::Client,
    secret: &str,
    response: &str,
    remote_ip: &IpAddr,
) -> Result<(), String> {
    if response.is_empty() {
        return Err("missing captcha_response".into());
    }
    // Try Cloudflare Turnstile first (most common for fresh deploys).
    let turnstile = client
        .post("https://challenges.cloudflare.com/turnstile/v0/siteverify")
        .form(&[
            ("secret", secret),
            ("response", response),
            ("remoteip", &remote_ip.to_string()),
        ])
        .send()
        .await;
    if let Ok(r) = turnstile {
        if r.status().is_success() {
            if let Ok(v) = r.json::<CaptchaSiteVerifyResponse>().await {
                if v.success {
                    return Ok(());
                }
            }
        }
    }
    // Fall through to hCaptcha.
    let hc = client
        .post("https://hcaptcha.com/siteverify")
        .form(&[
            ("secret", secret),
            ("response", response),
            ("remoteip", &remote_ip.to_string()),
        ])
        .send()
        .await;
    if let Ok(r) = hc {
        if r.status().is_success() {
            if let Ok(v) = r.json::<CaptchaSiteVerifyResponse>().await {
                if v.success {
                    return Ok(());
                }
            }
        }
    }
    Err("captcha verification failed".into())
}

// ─────────────────────────── App state ───────────────────────────────────

#[derive(Clone)]
struct AppState {
    cli: Arc<Cli>,
    store: Arc<Mutex<ClaimStore>>,
    http: reqwest::Client,
}

// ─────────────────────────── Handlers ────────────────────────────────────

async fn root() -> Html<&'static str> {
    Html(INDEX_HTML)
}

#[derive(Deserialize)]
struct ClaimReq {
    address: String,
    #[serde(default)]
    captcha_response: Option<String>,
}

#[derive(Serialize)]
struct ClaimResp {
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    balance: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

/// Forwarded response from the chain's /api/faucet.
#[derive(Deserialize)]
struct ChainFaucetResp {
    success: bool,
    #[serde(default)]
    balance: u64,
    #[serde(default)]
    message: Option<String>,
}

fn ok_resp(balance: u64, message: Option<String>) -> (StatusCode, Json<ClaimResp>) {
    (
        StatusCode::OK,
        Json(ClaimResp {
            success: true,
            balance: Some(balance),
            message,
        }),
    )
}

fn err_resp(code: StatusCode, msg: impl Into<String>) -> (StatusCode, Json<ClaimResp>) {
    (
        code,
        Json(ClaimResp {
            success: false,
            balance: None,
            message: Some(msg.into()),
        }),
    )
}

/// Pull the client IP from `X-Forwarded-For` if present (faucet expected
/// to run behind nginx/Caddy), else fall back to the connect-info socket.
fn client_ip(headers: &HeaderMap, fallback: SocketAddr) -> IpAddr {
    if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        // First entry, trimmed.
        if let Some(first) = xff.split(',').next() {
            if let Ok(ip) = first.trim().parse::<IpAddr>() {
                return ip;
            }
        }
    }
    if let Some(real) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
        if let Ok(ip) = real.trim().parse::<IpAddr>() {
            return ip;
        }
    }
    fallback.ip()
}

fn normalise_address(input: &str) -> Result<String, String> {
    let trimmed = input.trim().trim_start_matches("0x").to_lowercase();
    if trimmed.is_empty() {
        return Err("address is empty".into());
    }
    if !trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("address is not valid hex".into());
    }
    // Accept 32-byte (64-char) or 20-byte (40-char) addresses; the
    // chain's parse_address_value accepts either.
    if trimmed.len() != 64 && trimmed.len() != 40 {
        return Err(format!(
            "address must be 20 or 32 bytes hex (got {} chars)",
            trimmed.len()
        ));
    }
    Ok(trimmed)
}

async fn post_claim(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<ClaimReq>,
) -> impl IntoResponse {
    let ip = client_ip(&headers, addr);

    let normalised = match normalise_address(&req.address) {
        Ok(a) => a,
        Err(e) => return err_resp(StatusCode::BAD_REQUEST, format!("invalid address: {}", e)),
    };

    // CAPTCHA gate (only if --captcha-key was set).
    if let Some(ref secret) = state.cli.captcha_key {
        let resp = match req.captcha_response {
            Some(ref r) if !r.is_empty() => r.clone(),
            _ => return err_resp(StatusCode::BAD_REQUEST, "captcha_response required"),
        };
        if let Err(e) = verify_captcha(&state.http, secret, &resp, &ip).await {
            return err_resp(StatusCode::FORBIDDEN, e);
        }
    }

    // Rate-limit gate. Single Mutex covers the read-modify-write so
    // concurrent claims can't both squeak past the check.
    {
        let mut store = state.store.lock().await;
        if let Err(e) = check_and_record(
            &mut store,
            &ip,
            &normalised,
            state.cli.rate_limit_per_12h,
            now_unix_secs(),
        ) {
            return err_resp(StatusCode::TOO_MANY_REQUESTS, e);
        }
        if let Err(persist) = store.save(&state.cli.rate_limit_storage) {
            // Roll back the record so we don't book a claim we couldn't
            // persist. Best-effort: pop the most recent ts.
            let k = key(&ip, &normalised);
            if let Some(v) = store.entries.get_mut(&k) {
                v.pop();
                if store.total > 0 {
                    store.total -= 1;
                }
            }
            return err_resp(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("could not persist claim history: {}", persist),
            );
        }
    }

    // Forward to the chain.
    let url = format!("{}/api/faucet", state.cli.node_url.trim_end_matches('/'));
    let mut req_builder = state
        .http
        .post(&url)
        .json(&serde_json::json!({ "address": normalised }));
    if !state.cli.admin_token.is_empty() {
        req_builder = req_builder.bearer_auth(&state.cli.admin_token);
    }
    let chain_resp = match req_builder.send().await {
        Ok(r) => r,
        Err(e) => {
            return err_resp(
                StatusCode::BAD_GATEWAY,
                format!("could not reach node {}: {}", url, e),
            )
        }
    };
    let status = chain_resp.status();
    let body: ChainFaucetResp = match chain_resp.json().await {
        Ok(b) => b,
        Err(e) => {
            return err_resp(
                StatusCode::BAD_GATEWAY,
                format!("node returned non-JSON ({}): {}", status, e),
            )
        }
    };
    if !body.success {
        return err_resp(
            StatusCode::BAD_GATEWAY,
            body.message.unwrap_or_else(|| "node rejected claim".into()),
        );
    }
    ok_resp(body.balance, body.message)
}

#[derive(Serialize)]
struct HealthResp {
    ok: bool,
    claims_total: u64,
    claims_today: u64,
    rate_limit_per_12h: u32,
    max_claim_amount: u64,
    captcha_enabled: bool,
}

async fn get_health(State(state): State<AppState>) -> Json<HealthResp> {
    let store = state.store.lock().await;
    let now = now_unix_secs();
    Json(HealthResp {
        ok: true,
        claims_total: store.total,
        claims_today: claims_today(&store, now),
        rate_limit_per_12h: state.cli.rate_limit_per_12h,
        max_claim_amount: state.cli.max_claim_amount,
        captcha_enabled: state.cli.captcha_key.is_some(),
    })
}

// ─────────────────────────── Static HTML ─────────────────────────────────

const INDEX_HTML: &str = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>EvaporChain Testnet Faucet</title>
<style>
  :root {
    --bg: #fafbfc;
    --fg: #1a2233;
    --muted: #5a6478;
    --line: #e2e6ee;
    --accent: #1f6feb;
    --accent-hover: #1858c4;
    --ok: #118a4e;
    --err: #b3261e;
  }
  * { box-sizing: border-box; }
  html, body { height: 100%; }
  body {
    margin: 0;
    font: 15px/1.5 -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif;
    background: var(--bg);
    color: var(--fg);
    display: flex; align-items: center; justify-content: center;
  }
  main {
    max-width: 480px; width: 100%; padding: 32px;
    background: #fff; border: 1px solid var(--line); border-radius: 12px;
    margin: 24px;
  }
  h1 { margin: 0 0 4px; font-size: 22px; letter-spacing: -0.01em; }
  p.lede { margin: 0 0 20px; color: var(--muted); font-size: 13px; }
  label { display: block; margin: 14px 0 6px; font-size: 13px; color: var(--muted); }
  input[type=text] {
    width: 100%; padding: 10px 12px; font: inherit;
    border: 1px solid var(--line); border-radius: 8px;
    background: #fff; color: var(--fg);
  }
  input[type=text]:focus { outline: 2px solid var(--accent); border-color: var(--accent); }
  button {
    margin-top: 16px; width: 100%; padding: 11px 16px;
    font: inherit; font-weight: 600;
    background: var(--accent); color: #fff;
    border: 0; border-radius: 8px; cursor: pointer;
  }
  button:hover { background: var(--accent-hover); }
  button:disabled { opacity: 0.6; cursor: not-allowed; }
  #msg { margin-top: 16px; font-size: 13px; min-height: 18px; }
  #msg.ok { color: var(--ok); }
  #msg.err { color: var(--err); }
  footer { margin-top: 24px; font-size: 12px; color: var(--muted); text-align: center; }
  code { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 12px; }
</style>
</head>
<body>
<main>
  <h1>EvaporChain Testnet Faucet</h1>
  <p class="lede">Drop your testnet address to receive a top-up. Limited to one claim per address per IP per 12 hours.</p>
  <form id="f" autocomplete="off">
    <label for="addr">Testnet address</label>
    <input id="addr" name="address" type="text" placeholder="0x… (32-byte hex)" required>
    <button id="go" type="submit">Claim</button>
  </form>
  <div id="msg"></div>
  <footer>
    Need help? Check <code>/health</code> or run <code>curl -X POST -d '{"address":"…"}' …/claim</code>.
  </footer>
</main>
<script>
const f = document.getElementById("f");
const msg = document.getElementById("msg");
const go = document.getElementById("go");
f.addEventListener("submit", async (ev) => {
  ev.preventDefault();
  msg.textContent = ""; msg.className = "";
  go.disabled = true; const original = go.textContent; go.textContent = "Claiming…";
  try {
    const addr = document.getElementById("addr").value.trim();
    const r = await fetch("/claim", {
      method: "POST",
      headers: {"content-type": "application/json"},
      body: JSON.stringify({ address: addr })
    });
    const j = await r.json();
    if (j.success) {
      msg.className = "ok";
      msg.textContent = "Claimed. New balance: " + j.balance + " EVAP.";
    } else {
      msg.className = "err";
      msg.textContent = j.message || "Claim failed.";
    }
  } catch (e) {
    msg.className = "err";
    msg.textContent = "Network error: " + e;
  } finally {
    go.disabled = false; go.textContent = original;
  }
});
</script>
</body>
</html>"##;

// ─────────────────────────── Main ────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    use clap::Parser as _;
    let cli = Cli::parse();

    // Load existing claim history, if any.
    let store = ClaimStore::load(&cli.rate_limit_storage);
    let port = cli.port;
    let captcha_on = cli.captcha_key.is_some();
    let node_url = cli.node_url.clone();
    let limit = cli.rate_limit_per_12h;

    let state = AppState {
        cli: Arc::new(cli),
        store: Arc::new(Mutex::new(store)),
        http: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(8))
            .build()
            .context("build http client")?,
    };

    let app = Router::new()
        .route("/", get(root))
        .route("/claim", post(post_claim))
        .route("/health", get(get_health))
        .with_state(state);

    let bind = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .with_context(|| format!("bind {}", bind))?;

    println!(
        "evaporchain-faucet listening on http://0.0.0.0:{}\n  node:    {}\n  rate:    {} per 12h per (ip,addr)\n  captcha: {}",
        port,
        node_url,
        limit,
        if captcha_on { "on" } else { "off" }
    );

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .context("serve")?;
    Ok(())
}

// ─────────────────────────── Tests ────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn tmp_path(tag: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("faucet-{}-{}.json", tag, nonce))
    }

    #[test]
    fn rate_limit_blocks_second_claim_in_window() {
        let mut s = ClaimStore::default();
        let ip: IpAddr = Ipv4Addr::new(127, 0, 0, 1).into();
        let addr = "abcd";
        let now = 1_700_000_000;
        check_and_record(&mut s, &ip, addr, 1, now).expect("first claim allowed");
        let err = check_and_record(&mut s, &ip, addr, 1, now + 60)
            .expect_err("second within window must fail");
        assert!(err.contains("rate limited"));
    }

    #[test]
    fn rate_limit_lets_through_after_window() {
        let mut s = ClaimStore::default();
        let ip: IpAddr = Ipv4Addr::new(127, 0, 0, 1).into();
        let addr = "abcd";
        let now = 1_700_000_000;
        check_and_record(&mut s, &ip, addr, 1, now).unwrap();
        let later = now + WINDOW_SECS + 1;
        check_and_record(&mut s, &ip, addr, 1, later).expect("12h+ later must reset");
    }

    #[test]
    fn rate_limit_independent_per_ip() {
        let mut s = ClaimStore::default();
        let addr = "abcd";
        let now = 1_700_000_000;
        let a: IpAddr = Ipv4Addr::new(1, 1, 1, 1).into();
        let b: IpAddr = Ipv4Addr::new(2, 2, 2, 2).into();
        check_and_record(&mut s, &a, addr, 1, now).unwrap();
        check_and_record(&mut s, &b, addr, 1, now).expect("different IP unaffected");
    }

    #[test]
    fn rate_limit_independent_per_address() {
        let mut s = ClaimStore::default();
        let ip: IpAddr = Ipv4Addr::new(1, 1, 1, 1).into();
        let now = 1_700_000_000;
        check_and_record(&mut s, &ip, "aaaa", 1, now).unwrap();
        check_and_record(&mut s, &ip, "bbbb", 1, now).expect("different addr unaffected");
    }

    #[test]
    fn store_round_trips_through_disk() {
        let path = tmp_path("rt");
        let mut s = ClaimStore::default();
        let ip: IpAddr = Ipv4Addr::new(127, 0, 0, 1).into();
        check_and_record(&mut s, &ip, "abcd", 5, 1_700_000_000).unwrap();
        s.save(&path).unwrap();
        let loaded = ClaimStore::load(&path);
        assert_eq!(loaded.total, 1);
        assert_eq!(loaded.entries.len(), 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn normalise_address_accepts_20_and_32_bytes() {
        let s64 = "0x".to_string() + &"a".repeat(64);
        let s40 = "0x".to_string() + &"b".repeat(40);
        assert_eq!(normalise_address(&s64).unwrap().len(), 64);
        assert_eq!(normalise_address(&s40).unwrap().len(), 40);
        assert!(normalise_address("0xzzzz").is_err());
        assert!(normalise_address("").is_err());
        assert!(normalise_address("0xabcd").is_err());
    }

    #[test]
    fn claims_today_counts_only_within_24h_utc() {
        let mut s = ClaimStore::default();
        let ip: IpAddr = Ipv4Addr::new(127, 0, 0, 1).into();
        let yesterday = 1_700_000_000;
        let today = yesterday + 86_400 + 60;
        check_and_record(&mut s, &ip, "aaaa", 5, yesterday).unwrap();
        check_and_record(&mut s, &ip, "aaaa", 5, today).unwrap();
        let n = claims_today(&s, today);
        assert_eq!(n, 1, "only the today entry should be counted");
    }

    #[test]
    fn client_ip_prefers_xff_over_socket() {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", "203.0.113.5, 10.0.0.1".parse().unwrap());
        let sock: SocketAddr = "10.0.0.99:33333".parse().unwrap();
        let ip = client_ip(&h, sock);
        assert_eq!(ip.to_string(), "203.0.113.5");
    }

    #[test]
    fn client_ip_falls_back_to_socket() {
        let h = HeaderMap::new();
        let sock: SocketAddr = "10.0.0.99:33333".parse().unwrap();
        let ip = client_ip(&h, sock);
        assert_eq!(ip.to_string(), "10.0.0.99");
    }
}
