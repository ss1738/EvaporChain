//! Startup nonce reconciliation against the chain.
//!
//! Audit fix #3a (2026-05-09): the paymaster's local
//! `paymaster_nonce` file tracks "next nonce I'll assign". The chain's
//! `account.nonce` for the paymaster's address tracks "next nonce I
//! expect to see". After a service restart (graceful or post-crash) or
//! a chain reorg, the two can drift:
//!
//! - Local ahead: paymaster signed sponsorships that haven't yet landed
//!   on chain (mempool / network drop / orphaned). Refuse-and-warn:
//!   the operator may need to re-submit the in-flight UserOps before
//!   sponsoring more, or accept that those nonces will never be
//!   consumed (forever-gap if we keep going).
//!
//! - Chain ahead: someone else used the paymaster account, OR an
//!   earlier process used a higher nonce that we lost track of. Sync
//!   up: bump local to chain's value.
//!
//! - Aligned: nothing to do.
//!
//! This module does the **startup** reconciliation. Runtime reorg
//! handling (where the chain reorgs WHILE the paymaster is running) is
//! a separate piece (#3b in the audit) — not addressed here. The
//! startup case is the more common failure mode in practice (every
//! service restart, every redeploy).

use std::time::Duration;

use serde::Deserialize;
use thiserror::Error;

use evaporchain_types::AccountAddress;

/// Outcome of `check_alignment`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NonceAlignment {
    /// Local matches chain. Safe to start sponsoring.
    Aligned { nonce: u64 },
    /// Local file has assigned MORE nonces than the chain has consumed.
    /// Some prior sponsorships are unfinalised — in mempool, dropped, or
    /// reorged out. The paymaster MUST NOT keep allocating fresh nonces
    /// past the local file value, or it'll create a forever-gap of
    /// unusable nonces. Operator should investigate before starting.
    LocalAhead { local: u64, chain: u64 },
    /// Chain has consumed MORE nonces than the local file knows about.
    /// Either someone else used the paymaster account (which is a misuse
    /// — paymaster accounts should be sponsorship-only by convention)
    /// OR an earlier paymaster process advanced its nonce file and
    /// crashed before fsync (shouldn't happen given our atomic writes,
    /// but defensive). Resolution: bump the local file to match the
    /// chain.
    ChainAhead { local: u64, chain: u64 },
}

#[derive(Debug, Error)]
pub enum ReconcileError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("chain returned status {status}: {body}")]
    BadStatus { status: u16, body: String },
    #[error("invalid chain RPC URL: {0}")]
    InvalidUrl(String),
}

/// JSON shape of `GET /api/address/<addr>` — only the field we need.
#[derive(Debug, Deserialize)]
struct AddressDetail {
    nonce: u64,
}

/// Hit the chain's `GET /api/address/<paymaster_addr>` and compare
/// `account.nonce` against the paymaster's local "next nonce I'll
/// assign" value.
///
/// Semantics: `local_next_nonce` is what `Paymaster::next_paymaster_nonce()`
/// returns — the value that WOULD be assigned on the next call to
/// `sponsor`. The chain's `account.nonce` is what the chain expects
/// the next sponsored UserOp to carry as `paymaster_nonce`. These
/// should match in steady state.
pub async fn check_alignment(
    chain_rpc_url: &str,
    paymaster_address: AccountAddress,
    local_next_nonce: u64,
) -> Result<NonceAlignment, ReconcileError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;
    let addr_hex = format!("0x{}", hex::encode(paymaster_address));
    let url = format!(
        "{}/api/address/{}",
        chain_rpc_url.trim_end_matches('/'),
        addr_hex
    );
    let resp = client.get(&url).send().await?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(ReconcileError::BadStatus {
            status: status.as_u16(),
            body,
        });
    }
    let detail: AddressDetail = resp.json().await?;
    let chain_nonce = detail.nonce;

    Ok(match local_next_nonce.cmp(&chain_nonce) {
        std::cmp::Ordering::Equal => NonceAlignment::Aligned { nonce: chain_nonce },
        std::cmp::Ordering::Greater => NonceAlignment::LocalAhead {
            local: local_next_nonce,
            chain: chain_nonce,
        },
        std::cmp::Ordering::Less => NonceAlignment::ChainAhead {
            local: local_next_nonce,
            chain: chain_nonce,
        },
    })
}

/// One reconciliation cycle: query the chain, update the metrics,
/// log on drift. Returns the alignment outcome so the caller (e.g.,
/// a runtime poller) can react further.
///
/// Touches three metrics:
/// - `last_chain_nonce` (gauge) — set to the chain's value.
/// - `last_reconcile_unix_ms` (gauge) — set to current wall-clock.
/// - `drift_detections_total` (counter) — incremented on
///   non-Aligned outcomes.
///
/// Returns `Ok(NonceAlignment)` on a successful chain query (even
/// if the alignment shows drift). `Err(ReconcileError)` only on RPC
/// failure; in that case last_chain_nonce / last_reconcile are NOT
/// updated, and the operator can detect "RPC unreachable" by a
/// stale `last_reconcile_unix_ms` gauge.
pub async fn run_one_cycle(
    chain_rpc_url: &str,
    paymaster_address: AccountAddress,
    local_next_nonce: u64,
    metrics: &crate::PaymasterMetrics,
) -> Result<NonceAlignment, ReconcileError> {
    let outcome = check_alignment(chain_rpc_url, paymaster_address, local_next_nonce).await?;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    metrics
        .last_reconcile_unix_ms
        .store(now_ms, std::sync::atomic::Ordering::Relaxed);
    let chain_nonce = match &outcome {
        NonceAlignment::Aligned { nonce } => *nonce,
        NonceAlignment::LocalAhead { chain, .. } => *chain,
        NonceAlignment::ChainAhead { chain, .. } => *chain,
    };
    metrics
        .last_chain_nonce
        .store(chain_nonce, std::sync::atomic::Ordering::Relaxed);
    if !matches!(outcome, NonceAlignment::Aligned { .. }) {
        metrics
            .drift_detections_total
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::Path;
    use axum::routing::get;
    use axum::{Json, Router};
    use std::net::SocketAddr;
    use std::sync::Arc;

    /// Mock chain that returns a fixed `nonce` for any address query.
    async fn spawn_mock_chain(
        fixed_nonce: u64,
    ) -> (String, tokio::sync::oneshot::Sender<()>) {
        #[derive(serde::Serialize)]
        struct AddrResp {
            address: String,
            balance: u64,
            nonce: u64,
            objects: Vec<()>,
            nfts: Vec<()>,
            tokens: Vec<()>,
        }

        let nonce = Arc::new(fixed_nonce);
        async fn handler(
            Path(addr): Path<String>,
            axum::extract::State(n): axum::extract::State<Arc<u64>>,
        ) -> Json<AddrResp> {
            Json(AddrResp {
                address: addr,
                balance: 0,
                nonce: *n,
                objects: vec![],
                nfts: vec![],
                tokens: vec![],
            })
        }
        let app = Router::new()
            .route("/api/address/:addr", get(handler))
            .with_state(nonce);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.await;
                })
                .await
                .ok();
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        (format!("http://{addr}"), shutdown_tx)
    }

    #[tokio::test]
    async fn aligned_when_local_equals_chain() {
        let (url, _shutdown) = spawn_mock_chain(42).await;
        let pm_addr = [1u8; 32];
        let r = check_alignment(&url, pm_addr, 42).await.unwrap();
        assert_eq!(r, NonceAlignment::Aligned { nonce: 42 });
    }

    #[tokio::test]
    async fn local_ahead_when_local_greater() {
        let (url, _shutdown) = spawn_mock_chain(10).await;
        let pm_addr = [1u8; 32];
        let r = check_alignment(&url, pm_addr, 12).await.unwrap();
        assert_eq!(
            r,
            NonceAlignment::LocalAhead {
                local: 12,
                chain: 10,
            }
        );
    }

    #[tokio::test]
    async fn chain_ahead_when_chain_greater() {
        let (url, _shutdown) = spawn_mock_chain(99).await;
        let pm_addr = [1u8; 32];
        let r = check_alignment(&url, pm_addr, 50).await.unwrap();
        assert_eq!(
            r,
            NonceAlignment::ChainAhead {
                local: 50,
                chain: 99,
            }
        );
    }

    #[tokio::test]
    async fn errors_on_unreachable_chain() {
        // Bind a port and immediately drop the listener — port is closed.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let url = format!("http://{addr}");
        let pm_addr = [1u8; 32];
        let r = check_alignment(&url, pm_addr, 0).await;
        assert!(r.is_err(), "expected a connection error, got {r:?}");
    }

    // ─── Audit fix #3b: run_one_cycle metric updates ─────────────────

    #[tokio::test]
    async fn run_one_cycle_aligned_updates_gauges_no_drift_increment() {
        let (url, _shutdown) = spawn_mock_chain(7).await;
        let m = crate::PaymasterMetrics::default();
        let r = run_one_cycle(&url, [1u8; 32], 7, &m).await.unwrap();
        assert!(matches!(r, NonceAlignment::Aligned { nonce: 7 }));
        assert_eq!(m.last_chain_nonce.load(std::sync::atomic::Ordering::Relaxed), 7);
        assert!(m.last_reconcile_unix_ms.load(std::sync::atomic::Ordering::Relaxed) > 0);
        assert_eq!(m.drift_detections_total.load(std::sync::atomic::Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn run_one_cycle_drift_increments_counter() {
        let (url, _shutdown) = spawn_mock_chain(5).await;
        let m = crate::PaymasterMetrics::default();
        // Local at 12, chain at 5 → LocalAhead.
        let r = run_one_cycle(&url, [1u8; 32], 12, &m).await.unwrap();
        assert!(matches!(r, NonceAlignment::LocalAhead { local: 12, chain: 5 }));
        assert_eq!(m.drift_detections_total.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert_eq!(m.last_chain_nonce.load(std::sync::atomic::Ordering::Relaxed), 5);
    }

    #[tokio::test]
    async fn run_one_cycle_rpc_failure_doesnt_update_gauges() {
        // Bind+drop = closed port → connection refused.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let url = format!("http://{addr}");
        let m = crate::PaymasterMetrics::default();
        let r = run_one_cycle(&url, [1u8; 32], 7, &m).await;
        assert!(r.is_err());
        // Gauges stayed at default (0); operator detects unreachable
        // chain by `last_reconcile_unix_ms` not advancing.
        assert_eq!(m.last_chain_nonce.load(std::sync::atomic::Ordering::Relaxed), 0);
        assert_eq!(m.last_reconcile_unix_ms.load(std::sync::atomic::Ordering::Relaxed), 0);
        assert_eq!(m.drift_detections_total.load(std::sync::atomic::Ordering::Relaxed), 0);
    }

    /// T1.20 — check_alignment returns ReconcileError::BadStatus
    /// when the chain RPC returns a non-2xx (lines 97-101).
    /// Spins a mock chain that always 500s.
    #[tokio::test]
    async fn t1_20_check_alignment_bad_status_propagated() {
        async fn handler() -> (axum::http::StatusCode, &'static str) {
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "boom")
        }
        let app = Router::new().route("/api/address/:addr", get(handler));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.await;
                })
                .await
                .ok();
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        let url = format!("http://{addr}");

        let r = check_alignment(&url, [1u8; 32], 0).await;
        match r {
            Err(ReconcileError::BadStatus { status, .. }) => assert_eq!(status, 500),
            other => panic!("expected BadStatus, got {:?}", other),
        }
        let _ = shutdown_tx.send(());
    }
}
