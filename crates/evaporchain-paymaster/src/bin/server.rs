//! `evaporchain-paymaster` HTTP server.
//!
//! Day 2 of the Option B paymaster build (see
//! `docs/MULTI_TOKEN_GAS_OPTIONS.md`). Wallets POST a half-built
//! `UserOpTx` to `/sponsor`; the service stamps the paymaster's
//! sponsorship signature and returns the wire-ready tx.

use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::Parser;
use evaporchain_paymaster::{
    generate_keypair_to_file, load_keypair_from_file, InnerVariant, Paymaster,
    PaymasterConfig, PaymasterInfo, SponsorshipRequest, SponsorshipResponse,
};
use tracing::{error, info};

#[derive(Parser, Debug)]
#[command(name = "evaporchain-paymaster", about = "EvaporChain paymaster sponsorship service")]
struct Args {
    /// Path to the hybrid keypair JSON file. Generated on first run if
    /// missing AND --generate-keypair-if-missing is set.
    #[arg(long, default_value = "paymaster_keypair.json")]
    keypair_file: PathBuf,

    /// Path to the paymaster_nonce counter file. Atomically updated on
    /// each `/sponsor` call. Created on first run if missing.
    #[arg(long, default_value = "paymaster_nonce")]
    nonce_file: PathBuf,

    /// Chain ID — must match the running chain's chain_id, otherwise
    /// every sponsorship sig the service produces will be rejected on
    /// chain.
    #[arg(long)]
    chain_id: String,

    /// HTTP listen address.
    #[arg(long, default_value = "0.0.0.0:8088")]
    listen: String,

    /// First-run convenience: if the keypair file does not exist,
    /// generate one. Off by default so production deployments don't
    /// silently mint a fresh address.
    #[arg(long)]
    generate_keypair_if_missing: bool,

    /// Disable the user-signature pre-check (Day 7 hardening). Only
    /// safe for testnet / dev. Production paymasters should leave
    /// this off — the pre-check rejects spam-drain attempts before
    /// the paymaster spends gas on them.
    #[arg(long)]
    disable_user_sig_check: bool,

    /// Per-`UserOp.sender` token-bucket replenish rate, in
    /// sponsorships per second. `0` disables the rate limiter.
    /// Default `5.0`.
    #[arg(long, default_value = "5.0")]
    per_sender_rps: f64,

    /// Per-sender burst capacity. Default `10`.
    #[arg(long, default_value = "10")]
    per_sender_burst: u32,

    /// Append-only JSON-lines audit log path. Each successful
    /// sponsorship writes one line: `{ts_unix_ms, sender,
    /// paymaster_nonce, call_gas_limit, call_data_hash, chain_id}`.
    /// Used for billing reconciliation. Omitted means audit logging
    /// is disabled.
    #[arg(long)]
    audit_log: Option<PathBuf>,

    /// Audit fix #8a: fsync policy for the audit log. `per-line`
    /// (default) calls `sync_all` after every line — fail-closed
    /// durability, ~1k sponsorships/sec ceiling on standard SSDs.
    /// `none` skips the explicit fsync — ~10× throughput, but crash
    /// loss is bounded only by the OS dirty-page writeback schedule
    /// (typically ~30 s on Linux). Operators choosing `none` accept
    /// some recent audit lines may not survive a kernel crash.
    /// Group-commit (the safer middle option) is V1.5+.
    #[arg(long, default_value = "per-line")]
    audit_log_fsync: String,

    /// Operator-side inner-tx whitelist (Day 10). Comma-separated
    /// list of inner variants this paymaster will sponsor. Valid
    /// values: `transfer`, `call_script`, `call_contract`. Omitted
    /// means trust the chain (sponsor any chain-accepted variant).
    /// Example: `--allow-inner=transfer` for a transfer-only
    /// paymaster; `--allow-inner=transfer,call_script` to allow
    /// both. Empty `call_data` (gas-only sponsorship) is always
    /// allowed.
    #[arg(long, value_delimiter = ',')]
    allow_inner: Vec<String>,

    /// Day 12 — idempotency cache size. Wallets that send an
    /// `Idempotency-Key` header on `/sponsor` and retry under the
    /// same key get the cached response (same paymaster_nonce,
    /// same sig). `0` disables the cache. Default `1024`.
    #[arg(long, default_value = "1024")]
    idempotency_max_keys: usize,

    /// Day 12 — idempotency cache TTL in seconds. Default `3600`
    /// (1h). Tune higher for wallets that may retry across longer
    /// periods (e.g. user laptop sleeping mid-flight).
    #[arg(long, default_value = "3600")]
    idempotency_ttl_secs: u64,

    /// Audit fix #6a: on-disk path for the idempotency cache.
    /// Loaded at startup; re-written on every successful insert
    /// (atomic temp+rename). Wallet retries spanning a paymaster
    /// restart still get cache-replay. Omit to keep the legacy
    /// in-memory-only behavior.
    #[arg(long)]
    idempotency_persist_path: Option<PathBuf>,

    /// Audit fix #3a (2026-05-09): chain RPC URL for startup nonce
    /// reconciliation. When set, the paymaster fetches its own
    /// `account.nonce` from `/api/address/<addr>` and compares
    /// against the local `paymaster_nonce` file. Loud warning on
    /// drift (LocalAhead = unfinalised sponsorships,
    /// ChainAhead = someone else used the account or stale local
    /// file). Reconciliation outcome is logged but doesn't fail
    /// startup unless `--strict-reconcile` is also set.
    #[arg(long)]
    chain_rpc_url: Option<String>,

    /// If set with `--chain-rpc-url`, refuse to start when local
    /// `paymaster_nonce` file is ahead of chain. Production
    /// deployments should set this — sponsoring while local-ahead
    /// either creates a forever-gap of unusable nonces (if local was
    /// right and chain just hasn't seen the in-flight UserOps yet)
    /// or duplicates already-consumed nonces (if a prior process
    /// crashed mid-write). Operator must investigate either way.
    #[arg(long, requires = "chain_rpc_url")]
    strict_reconcile: bool,

    /// Audit fix #3b: runtime reconciliation poll interval (seconds).
    /// `0` disables the runtime poller (startup reconciliation still
    /// runs once when `--chain-rpc-url` is set). Default `60` —
    /// catches drift within a minute of a reorg landing on chain.
    /// Requires `--chain-rpc-url`.
    #[arg(long, default_value = "60", requires = "chain_rpc_url")]
    reconcile_interval_secs: u64,
}

#[derive(Clone)]
struct AppState {
    paymaster: Arc<Paymaster>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let args = Args::parse();

    let keypair = if args.keypair_file.exists() {
        info!(path = %args.keypair_file.display(), "loading paymaster keypair");
        load_keypair_from_file(&args.keypair_file)?
    } else if args.generate_keypair_if_missing {
        info!(path = %args.keypair_file.display(), "generating new paymaster keypair");
        generate_keypair_to_file(&args.keypair_file)?
    } else {
        anyhow::bail!(
            "paymaster keypair file {} does not exist; pass \
             --generate-keypair-if-missing to mint one",
            args.keypair_file.display()
        );
    };

    let allowed_inner_variants = if args.allow_inner.is_empty() {
        None
    } else {
        let mut parsed = Vec::with_capacity(args.allow_inner.len());
        for raw in &args.allow_inner {
            let v = InnerVariant::parse_cli(raw.trim()).ok_or_else(|| {
                anyhow::anyhow!(
                    "--allow-inner: unknown inner variant '{raw}' \
                     (valid: transfer, call_script, call_contract)"
                )
            })?;
            parsed.push(v);
        }
        Some(parsed)
    };
    let audit_log_fsync = match args.audit_log_fsync.as_str() {
        "per-line" | "per_line" => evaporchain_paymaster::AuditFsyncMode::PerLine,
        "none" => evaporchain_paymaster::AuditFsyncMode::None,
        other => {
            anyhow::bail!(
                "--audit-log-fsync: unknown mode '{other}' (valid: per-line, none)"
            );
        }
    };
    let config = PaymasterConfig {
        require_user_sig: !args.disable_user_sig_check,
        per_sender_rps: args.per_sender_rps,
        per_sender_burst: args.per_sender_burst,
        audit_log: args.audit_log.clone(),
        audit_log_fsync,
        allowed_inner_variants,
        idempotency_max_keys: args.idempotency_max_keys,
        idempotency_ttl_secs: args.idempotency_ttl_secs,
        idempotency_persist_path: args.idempotency_persist_path.clone(),
    };
    let paymaster = Paymaster::new_with_config(
        keypair,
        args.chain_id.clone(),
        args.nonce_file.clone(),
        config,
    )?;
    let info = paymaster.info();
    info!(
        paymaster_address = %info.paymaster_address_hex,
        chain_id = %info.chain_id,
        next_nonce = info.next_paymaster_nonce,
        "paymaster ready"
    );

    // Audit fix #3a — startup nonce reconciliation against the chain.
    if let Some(ref chain_rpc) = args.chain_rpc_url {
        use evaporchain_paymaster::reconcile::{check_alignment, NonceAlignment};
        match check_alignment(chain_rpc, paymaster.address(), info.next_paymaster_nonce).await {
            Ok(NonceAlignment::Aligned { nonce }) => {
                info!(nonce, "nonce reconciliation: local matches chain");
            }
            Ok(NonceAlignment::LocalAhead { local, chain }) => {
                error!(
                    local,
                    chain,
                    "nonce reconciliation: LOCAL AHEAD by {} — unfinalised sponsorships in mempool, dropped, or reorged out. Investigate before continuing.",
                    local - chain
                );
                if args.strict_reconcile {
                    anyhow::bail!(
                        "refusing to start: --strict-reconcile is set and local nonce file is ahead of chain (local={local}, chain={chain})"
                    );
                }
            }
            Ok(NonceAlignment::ChainAhead { local, chain }) => {
                error!(
                    local,
                    chain,
                    "nonce reconciliation: CHAIN AHEAD by {} — someone else used the paymaster account or local file is stale. Manual fix: stop the service, write {} into the nonce file, restart.",
                    chain - local,
                    chain
                );
                if args.strict_reconcile {
                    anyhow::bail!(
                        "refusing to start: --strict-reconcile is set and chain nonce is ahead of local (local={local}, chain={chain})"
                    );
                }
            }
            Err(e) => {
                error!(
                    error = %e,
                    "nonce reconciliation failed — could not query chain. Continuing without reconciliation; consider checking chain RPC connectivity"
                );
                if args.strict_reconcile {
                    anyhow::bail!(
                        "refusing to start: --strict-reconcile is set and reconciliation RPC failed: {e}"
                    );
                }
            }
        }
    }

    let state = AppState {
        paymaster: Arc::new(paymaster),
    };

    // Audit fix #3b — runtime reconciliation poller. Periodic chain
    // query; updates metrics; logs on drift. Doesn't refuse /sponsor
    // (operator response is external — page on metrics, take action).
    if let Some(ref chain_rpc) = args.chain_rpc_url {
        if args.reconcile_interval_secs > 0 {
            spawn_reconcile_poller(
                state.paymaster.clone(),
                chain_rpc.clone(),
                args.reconcile_interval_secs,
            );
        }
    }

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/info", get(get_info))
        .route("/metrics", get(get_metrics))
        .route("/sponsor", post(sponsor))
        .with_state(state.clone());

    // Day 13 — SIGHUP triggers audit-log reopen. Operator workflow:
    //   mv audit.jsonl audit-$(date +%F).jsonl
    //   kill -HUP <pid>
    // Without this, sponsorships keep landing in the renamed file
    // (kernel keeps the old inode alive while we hold the fd) and
    // the fresh `audit.jsonl` stays empty until restart.
    #[cfg(unix)]
    spawn_sighup_listener(state.paymaster.clone());

    let listener = tokio::net::TcpListener::bind(&args.listen).await?;
    info!(addr = %args.listen, "paymaster listening");
    axum::serve(listener, app).await?;
    Ok(())
}

/// Audit fix #3b: tokio task that periodically calls
/// `reconcile::run_one_cycle` to detect runtime nonce drift. On
/// drift, logs at `error!` so the operator's log alerting can fire.
/// Metrics (`drift_detections_total`, `last_chain_nonce`,
/// `last_reconcile_unix_ms`) are updated inside `run_one_cycle`.
fn spawn_reconcile_poller(
    paymaster: Arc<Paymaster>,
    chain_rpc_url: String,
    interval_secs: u64,
) {
    use evaporchain_paymaster::reconcile::{run_one_cycle, NonceAlignment};
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
        // Skip the immediate-fire of the first tick — startup
        // reconciliation already ran once.
        tick.tick().await;
        loop {
            tick.tick().await;
            let local = paymaster.next_paymaster_nonce();
            match run_one_cycle(
                &chain_rpc_url,
                paymaster.address(),
                local,
                paymaster.metrics(),
            )
            .await
            {
                Ok(NonceAlignment::Aligned { .. }) => {
                    // Aligned is the steady state; quiet.
                }
                Ok(NonceAlignment::LocalAhead { local, chain }) => {
                    error!(
                        local,
                        chain,
                        "runtime drift: LOCAL AHEAD by {} — sponsorships in flight or chain reorged out",
                        local - chain
                    );
                }
                Ok(NonceAlignment::ChainAhead { local, chain }) => {
                    error!(
                        local,
                        chain,
                        "runtime drift: CHAIN AHEAD by {} — paymaster account used externally OR local file lost state",
                        chain - local
                    );
                }
                Err(e) => {
                    error!(error = %e, "runtime reconcile cycle failed — chain RPC unreachable");
                }
            }
        }
    });
}

#[cfg(unix)]
fn spawn_sighup_listener(paymaster: Arc<Paymaster>) {
    tokio::spawn(async move {
        let mut sighup = match tokio::signal::unix::signal(
            tokio::signal::unix::SignalKind::hangup(),
        ) {
            Ok(s) => s,
            Err(e) => {
                error!(error = %e, "failed to register SIGHUP handler — audit log rotation will require restart");
                return;
            }
        };
        loop {
            sighup.recv().await;
            match paymaster.reopen_audit_log() {
                Ok(true) => info!("SIGHUP — audit log reopened"),
                Ok(false) => {
                    info!("SIGHUP received but audit logging is disabled — no-op")
                }
                Err(e) => {
                    error!(error = %e, "SIGHUP — audit log reopen failed; subsequent sponsorships will fail with audit IO until fixed")
                }
            }
        }
    });
}

async fn healthz() -> &'static str {
    "ok"
}

async fn get_info(State(state): State<AppState>) -> Json<PaymasterInfo> {
    Json(state.paymaster.info())
}

/// Prometheus exposition. Content-Type per the Prometheus spec is
/// `text/plain; version=0.0.4`. Scrapers (Prometheus, vmagent,
/// vector, etc.) hit this on a fixed cadence (15s default) — keep
/// the response cheap.
async fn get_metrics(State(state): State<AppState>) -> Response {
    let body = state.paymaster.prometheus_metrics();
    (
        StatusCode::OK,
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4",
        )],
        body,
    )
        .into_response()
}

async fn sponsor(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<SponsorshipRequest>,
) -> Result<Json<SponsorshipResponse>, AppError> {
    // Day 12 — wallets can opt into idempotent retry by sending an
    // `Idempotency-Key: <opaque>` header (HTTP convention; see
    // draft-ietf-httpapi-idempotency-key-header). The paymaster
    // caches the response keyed on this string for `idempotency_ttl`,
    // so a wallet retry returns the same paymaster_nonce + sig
    // instead of a fresh allocation.
    let idempotency_key = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let mut user_op = req.user_op;
    let paymaster_addr = state.paymaster.address();
    let outcome = state
        .paymaster
        .sponsor_idempotent(idempotency_key.as_deref(), &mut user_op)
        .map_err(|e| {
            error!(error = %e, "sponsor failed");
            AppError::from_paymaster(e)
        })?;
    Ok(Json(SponsorshipResponse {
        user_op,
        paymaster_address_hex: hex::encode(paymaster_addr),
        paymaster_nonce: outcome.paymaster_nonce(),
    }))
}

#[derive(Debug, thiserror::Error)]
enum AppError {
    /// Catch-all for sponsorship-side rejections: malformed request,
    /// already-signed UserOp, missing/invalid user signature, etc.
    /// Surfaced as `400 Bad Request`.
    #[error("sponsor failed: {0}")]
    SponsorFailed(String),
    /// Per-`UserOp.sender` rate limit hit. Surfaced as
    /// `429 Too Many Requests` so wallets can back off cleanly.
    #[error("rate limited: {0}")]
    RateLimited(String),
    /// Underlying nonce-file IO error. Surfaced as
    /// `503 Service Unavailable` — the paymaster can't safely sponsor
    /// without persisting nonce state.
    #[error("paymaster IO: {0}")]
    Io(String),
}

impl AppError {
    fn from_paymaster(e: evaporchain_paymaster::PaymasterError) -> Self {
        use evaporchain_paymaster::PaymasterError;
        match e {
            PaymasterError::RateLimited { sender_hex } => Self::RateLimited(sender_hex),
            PaymasterError::NonceIo(e) => Self::Io(e.to_string()),
            // Audit-IO surfaces as 503 too — the operator can't bill
            // a sponsorship they couldn't audit, so the wallet should
            // not retry until the operator unblocks.
            PaymasterError::AuditIo(msg) => Self::Io(format!("audit log: {msg}")),
            other => Self::SponsorFailed(other.to_string()),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let body = serde_json::json!({"error": self.to_string()}).to_string();
        let status = match self {
            AppError::SponsorFailed(_) => StatusCode::BAD_REQUEST,
            AppError::RateLimited(_) => StatusCode::TOO_MANY_REQUESTS,
            AppError::Io(_) => StatusCode::SERVICE_UNAVAILABLE,
        };
        (status, [(axum::http::header::CONTENT_TYPE, "application/json")], body).into_response()
    }
}
