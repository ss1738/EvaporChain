//! `evaporchain-paymaster` HTTP server.
//!
//! Day 2 of the Option B paymaster build (see
//! `docs/MULTI_TOKEN_GAS_OPTIONS.md`). Wallets POST a half-built
//! `UserOpTx` to `/sponsor`; the service stamps the paymaster's
//! sponsorship signature and returns the wire-ready tx.

use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::Parser;
use evaporchain_paymaster::{
    generate_keypair_to_file, load_keypair_from_file, Paymaster, PaymasterInfo,
    SponsorshipRequest, SponsorshipResponse,
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

    let paymaster = Paymaster::new(keypair, args.chain_id.clone(), args.nonce_file.clone())?;
    let info = paymaster.info();
    info!(
        paymaster_address = %info.paymaster_address_hex,
        chain_id = %info.chain_id,
        next_nonce = info.next_paymaster_nonce,
        "paymaster ready"
    );

    let state = AppState {
        paymaster: Arc::new(paymaster),
    };

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/info", get(get_info))
        .route("/sponsor", post(sponsor))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&args.listen).await?;
    info!(addr = %args.listen, "paymaster listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn healthz() -> &'static str {
    "ok"
}

async fn get_info(State(state): State<AppState>) -> Json<PaymasterInfo> {
    Json(state.paymaster.info())
}

async fn sponsor(
    State(state): State<AppState>,
    Json(req): Json<SponsorshipRequest>,
) -> Result<Json<SponsorshipResponse>, AppError> {
    let mut user_op = req.user_op;
    let paymaster_addr = state.paymaster.address();
    let assigned_nonce = state.paymaster.sponsor(&mut user_op).map_err(|e| {
        error!(error = %e, "sponsor failed");
        AppError::SponsorFailed(e.to_string())
    })?;
    Ok(Json(SponsorshipResponse {
        user_op,
        paymaster_address_hex: hex::encode(paymaster_addr),
        paymaster_nonce: assigned_nonce,
    }))
}

#[derive(Debug, thiserror::Error)]
enum AppError {
    #[error("sponsor failed: {0}")]
    SponsorFailed(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let body = serde_json::json!({"error": self.to_string()}).to_string();
        let status = match self {
            AppError::SponsorFailed(_) => StatusCode::BAD_REQUEST,
        };
        (status, [(axum::http::header::CONTENT_TYPE, "application/json")], body).into_response()
    }
}
