// wallet/src/paymaster.rs — client for evaporchain-paymaster sponsorship service
//
// Day 3 of the Option B paymaster build (see
// docs/MULTI_TOKEN_GAS_OPTIONS.md). Pairs with:
//   - chain-side enforcement: dc89531 (sponsorship sig), 3ccf4f7 (call_data dispatch)
//   - paymaster service:      cd64a3b (HTTP server)
//
// This module gives wallet code a thin async client for the paymaster
// service. Wallets use it to:
//
//   1. Discover a paymaster's address + next-nonce via GET /info.
//   2. Build a half-formed UserOpTx encoding their inner intent.
//   3. POST the UserOpTx to /sponsor; the paymaster stamps its
//      sponsorship signature and returns the wire-ready UserOp.
//   4. Submit the returned UserOp to the chain via the existing
//      `rpc::RpcClient::submit_user_op` (or equivalent) endpoint.
//
// The wallet does NOT submit the inner Transaction directly — the
// chain-side `execute_user_op` runs the inner intent against the
// sender's balance with the paymaster paying gas. See
// `docs/MULTI_TOKEN_GAS_OPTIONS.md` §3 for why this is the
// envelope-route default.

use evaporchain_paymaster::{PaymasterInfo, SponsorshipRequest, SponsorshipResponse};
use evaporchain_types::{AccountAddress, Transaction, UserOpTx};
use thiserror::Error;
use url::Url;

#[derive(Debug, Error)]
pub enum PaymasterClientError {
    #[error("invalid paymaster URL: {0}")]
    InvalidUrl(String),
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("paymaster returned status {status}: {body}")]
    BadStatus { status: u16, body: String },
    #[error("call_data encoding failed: {0}")]
    CallDataEncoding(#[from] serde_json::Error),
}

/// Async client for a single paymaster service.
///
/// The client is cheap to clone (`reqwest::Client` is `Arc<...>`
/// under the hood) — share one per wallet across requests rather
/// than building per-call.
#[derive(Clone)]
pub struct PaymasterClient {
    base_url: Url,
    http: reqwest::Client,
}

impl PaymasterClient {
    /// Build a client targeting `base_url` (e.g. `http://paymaster.example:8088`).
    pub fn new(base_url: &str) -> Result<Self, PaymasterClientError> {
        let base_url = Url::parse(base_url)
            .map_err(|e| PaymasterClientError::InvalidUrl(e.to_string()))?;
        Ok(Self {
            base_url,
            http: reqwest::Client::new(),
        })
    }

    /// Build a client with a pre-configured `reqwest::Client`. Useful
    /// when the wallet wants shared connection pooling, custom timeouts,
    /// or proxy settings across multiple paymasters.
    pub fn with_http_client(
        base_url: &str,
        http: reqwest::Client,
    ) -> Result<Self, PaymasterClientError> {
        let base_url = Url::parse(base_url)
            .map_err(|e| PaymasterClientError::InvalidUrl(e.to_string()))?;
        Ok(Self { base_url, http })
    }

    /// `GET /info` — the paymaster's address, next sponsorship nonce,
    /// and chain_id. Wallets use this to:
    ///   - confirm the paymaster targets the right chain
    ///   - read `next_paymaster_nonce` so the wallet can stamp
    ///     `paymaster_nonce` on the UserOp before signing the user side
    ///     (the paymaster will overwrite it on `/sponsor` with its
    ///     authoritative value, but pre-stamping lets the user's
    ///     signable bytes include the nonce)
    pub async fn info(&self) -> Result<PaymasterInfo, PaymasterClientError> {
        let url = self
            .base_url
            .join("info")
            .map_err(|e| PaymasterClientError::InvalidUrl(e.to_string()))?;
        let resp = self.http.get(url).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(PaymasterClientError::BadStatus {
                status: status.as_u16(),
                body,
            });
        }
        Ok(resp.json().await?)
    }

    /// `POST /sponsor` — ask the paymaster to stamp its sponsorship
    /// signature on `user_op`. The paymaster overwrites `paymaster`,
    /// `paymaster_nonce`, `paymaster_signature`, and
    /// `paymaster_public_key`; everything else round-trips unchanged.
    pub async fn sponsor(
        &self,
        user_op: UserOpTx,
    ) -> Result<SponsorshipResponse, PaymasterClientError> {
        let url = self
            .base_url
            .join("sponsor")
            .map_err(|e| PaymasterClientError::InvalidUrl(e.to_string()))?;
        let req = SponsorshipRequest { user_op };
        let resp = self.http.post(url).json(&req).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(PaymasterClientError::BadStatus {
                status: status.as_u16(),
                body,
            });
        }
        Ok(resp.json().await?)
    }
}

/// Build the half-formed `UserOpTx` a wallet sends to a paymaster.
///
/// Inputs:
/// - `sender` — the user's account address.
/// - `sender_nonce` — the user's next account nonce (caller must
///   read this from chain state; see `wallet::nonce` module).
/// - `inner` — the user's actual intent (e.g.
///   `Transaction::Transfer(...)`). For V1 only `Transfer` is
///   accepted by the chain's `execute_user_op` whitelist.
/// - `call_gas_limit` — gas budget the paymaster commits to. The
///   chain debits `call_gas_limit + GAS_USER_OP` from the paymaster.
///
/// The returned `UserOpTx` has paymaster fields = None; the
/// paymaster service fills those in. The user's own signature on
/// the UserOp body (the `signature` / `public_key` fields) is the
/// caller's responsibility — the wallet's `signer` module signs
/// `Transaction::UserOp(user_op).signing_message(chain_id)` and
/// stamps the result.
pub fn build_unsigned_user_op(
    sender: AccountAddress,
    sender_nonce: u64,
    inner: &Transaction,
    call_gas_limit: u64,
) -> Result<UserOpTx, PaymasterClientError> {
    let call_data = serde_json::to_vec(inner)?;
    Ok(UserOpTx {
        sender,
        nonce: sender_nonce,
        call_data,
        call_gas_limit,
        paymaster: None,
        paymaster_nonce: None,
        paymaster_data: None,
        paymaster_signature: None,
        paymaster_public_key: None,
        signature: None,
        public_key: None,
    })
}

/// Stamp the user's (sender's) signature on a `UserOpTx`. This is the
/// authorisation half of a sponsored UserOp — the chain's
/// `verify_tx_signature` checks this signature against
/// `Transaction::UserOp(user_op).signing_message(chain_id)` (which
/// prefixes the tx body with the chain_id length + chain_id bytes for
/// EIP-155-style cross-chain replay protection).
///
/// Order matters: stamp the user signature **before** sending to the
/// paymaster, OR after — both work. The user's signature covers
/// `signable_bytes`, which includes `paymaster` (when set) but not
/// `paymaster_nonce` / `paymaster_signature` / `paymaster_public_key`,
/// so the paymaster can fill those in without invalidating the user
/// sig. The paymaster signature, conversely, binds
/// `blake3(call_data)` and is stamped by the paymaster service —
/// neither side can tamper with the other's commitments.
///
/// Why this lives in `wallet::paymaster` rather than `wallet::signer`:
/// the existing `WalletSigner::sign_transaction` signs over
/// `signable_bytes` only (no chain_id prefix) — a pre-existing
/// wallet bug that's harmless when the chain runs with
/// `verify_signatures = false`. Sponsored UserOps go to a
/// `verify_signatures = true` path on V1 mainnet, so we need a
/// chain-id-aware signer specifically here. The wallet's general
/// chain-id awareness is its own follow-up.
pub fn sign_user_op_as_sender(
    user_op: &mut UserOpTx,
    chain_id: &str,
    signer_keypair: &evaporchain_crypto::signatures::HybridKeypair,
) {
    use evaporchain_crypto::signatures::Signer as _;
    // Build the canonical message the chain will verify against. We
    // construct a `Transaction::UserOp(user_op.clone())` so the
    // existing `signing_message` routes through the right enum arm
    // and produces the canonical `0x12` type-tagged signable bytes.
    let canonical_tx = Transaction::UserOp(user_op.clone());
    let msg = canonical_tx.signing_message(chain_id);
    let sig = signer_keypair.sign(&msg);
    let pk = signer_keypair.public_key_bytes();
    user_op.signature = Some(sig);
    user_op.public_key = Some(pk);
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_crypto::signatures::{HybridKeypair, HybridVerifier, Signer, Verifier};
    use evaporchain_paymaster::{generate_keypair_to_file, Paymaster, PaymasterInfo};
    use evaporchain_types::TransferTx;
    use std::sync::Arc;

    /// Spawn the real paymaster HTTP server in a tokio task and return
    /// (PaymasterClient pointing at it, paymaster_address, shutdown
    /// trigger). Caller drops the trigger to shut the server down.
    async fn spawn_paymaster_for_test(
        chain_id: &str,
    ) -> (
        PaymasterClient,
        AccountAddress,
        tokio::sync::oneshot::Sender<()>,
    ) {
        use axum::{
            extract::State, http::StatusCode, routing::{get, post}, Json, Router,
        };

        let tmp = tempfile::TempDir::new().unwrap();
        let nonce_file = tmp.path().join("paymaster_nonce");
        // Keep the temp dir alive for the duration of the test by
        // leaking it. The OS will reap the inode on the next reboot;
        // the test process itself drops it once the test exits.
        Box::leak(Box::new(tmp));

        let kp = HybridKeypair::generate();
        let paymaster = Arc::new(
            Paymaster::new(kp, chain_id.to_string(), nonce_file).expect("paymaster"),
        );
        let pm_addr = paymaster.address();

        #[derive(Clone)]
        struct AppState {
            paymaster: Arc<Paymaster>,
        }
        async fn get_info(State(state): State<AppState>) -> Json<PaymasterInfo> {
            Json(state.paymaster.info())
        }
        async fn sponsor(
            State(state): State<AppState>,
            Json(req): Json<SponsorshipRequest>,
        ) -> Result<Json<SponsorshipResponse>, (StatusCode, String)> {
            let mut user_op = req.user_op;
            let assigned = state.paymaster.sponsor(&mut user_op).map_err(
                |e: evaporchain_paymaster::PaymasterError| {
                    (StatusCode::BAD_REQUEST, e.to_string())
                },
            )?;
            Ok(Json(SponsorshipResponse {
                user_op,
                paymaster_address_hex: hex::encode(state.paymaster.address()),
                paymaster_nonce: assigned,
            }))
        }

        let app = Router::new()
            .route("/info", get(get_info))
            .route("/sponsor", post(sponsor))
            .with_state(AppState { paymaster });

        // Bind ephemeral port; capture it before serving.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let local_addr = listener.local_addr().unwrap();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.await;
                })
                .await
                .ok();
        });

        // Give the listener a moment to start accepting.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let client =
            PaymasterClient::new(&format!("http://{local_addr}/")).expect("paymaster client");
        (client, pm_addr, shutdown_tx)
    }

    #[tokio::test]
    async fn info_round_trip() {
        let (client, pm_addr, _shutdown) = spawn_paymaster_for_test("test-chain").await;
        let info = client.info().await.expect("/info");
        assert_eq!(info.paymaster_address_hex, hex::encode(pm_addr));
        assert_eq!(info.next_paymaster_nonce, 0);
        assert_eq!(info.chain_id, "test-chain");
    }

    #[tokio::test]
    async fn sponsor_returns_chain_acceptable_user_op() {
        let chain_id = "evaporchain-mainnet";
        let (client, pm_addr, _shutdown) = spawn_paymaster_for_test(chain_id).await;

        let sender: AccountAddress = [1u8; 32];
        let inner = Transaction::Transfer(TransferTx {
            from: sender,
            to: [9u8; 32],
            amount: 500,
            nonce: 0,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        });
        let user_op = build_unsigned_user_op(sender, 0, &inner, 50_000).expect("build user op");

        let resp = client.sponsor(user_op).await.expect("/sponsor");

        assert_eq!(resp.paymaster_address_hex, hex::encode(pm_addr));
        assert_eq!(resp.paymaster_nonce, 0);
        let returned = resp.user_op;
        assert_eq!(returned.paymaster, Some(pm_addr));
        assert_eq!(returned.paymaster_nonce, Some(0));

        // The same two checks `execute_user_op` runs at chain time:
        // (a) blake3(pk) derives to paymaster_address.
        let pk = returned.paymaster_public_key.as_deref().unwrap();
        let derived: AccountAddress = *blake3::hash(pk).as_bytes();
        assert_eq!(derived, pm_addr);
        // (b) sig verifies under the canonical sponsorship payload.
        let payload = returned
            .paymaster_sponsorship_payload(chain_id)
            .expect("payload");
        let sig = returned.paymaster_signature.as_deref().unwrap();
        assert!(
            HybridVerifier::verify(&payload, sig, pk),
            "round-tripped UserOp must satisfy chain rules"
        );
    }

    #[tokio::test]
    async fn sponsor_assigns_monotonic_nonces_across_calls() {
        let (client, _, _shutdown) = spawn_paymaster_for_test("test").await;
        let sender: AccountAddress = [1u8; 32];
        let inner = Transaction::Transfer(TransferTx {
            from: sender,
            to: [9u8; 32],
            amount: 1,
            nonce: 0,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        });

        let mut nonces = Vec::new();
        for _ in 0..3 {
            let uo = build_unsigned_user_op(sender, 0, &inner, 1000).unwrap();
            let resp = client.sponsor(uo).await.unwrap();
            nonces.push(resp.paymaster_nonce);
        }
        assert_eq!(nonces, vec![0, 1, 2]);
    }

    #[tokio::test]
    async fn sponsor_refuses_already_signed_user_op() {
        let (client, _, _shutdown) = spawn_paymaster_for_test("test").await;
        let sender: AccountAddress = [1u8; 32];
        let inner = Transaction::Transfer(TransferTx {
            from: sender,
            to: [9u8; 32],
            amount: 1,
            nonce: 0,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        });
        let mut uo = build_unsigned_user_op(sender, 0, &inner, 1000).unwrap();
        uo.paymaster_signature = Some(vec![0u8; 32]);

        let r = client.sponsor(uo).await;
        assert!(matches!(
            r,
            Err(PaymasterClientError::BadStatus { status: 400, .. })
        ));
    }

    #[test]
    fn build_unsigned_user_op_is_round_trippable() {
        // The chain decodes call_data via serde_json::from_slice; we
        // encode the same way. Round-trip in the test process to lock
        // the wire shape against silent regressions.
        let sender: AccountAddress = [7u8; 32];
        let inner = Transaction::Transfer(TransferTx {
            from: sender,
            to: [8u8; 32],
            amount: 42,
            nonce: 0,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        });
        let uo = build_unsigned_user_op(sender, 5, &inner, 21_000).unwrap();
        let decoded: Transaction = serde_json::from_slice(&uo.call_data).unwrap();
        match decoded {
            Transaction::Transfer(t) => {
                assert_eq!(t.from, sender);
                assert_eq!(t.amount, 42);
            }
            _ => panic!("expected Transfer"),
        }
    }

    #[tokio::test]
    async fn double_signed_user_op_satisfies_both_chain_checks() {
        // Lock the doubly-signed flow:
        //   1. Wallet picks the paymaster, builds half-formed UserOp
        //      with paymaster + paymaster_nonce stamped (read from
        //      /info), and the user's signature on the UserOp body.
        //   2. POSTs to /sponsor; paymaster stamps its sponsorship sig
        //      over the canonical sponsorship payload (paymaster_*
        //      fields).
        //   3. Returned UserOp has BOTH signatures. Each verifies
        //      against its own canonical message:
        //        - user sig verifies against
        //          Transaction::UserOp.signing_message(chain_id)
        //        - paymaster sig verifies against
        //          UserOpTx::paymaster_sponsorship_payload(chain_id)
        let chain_id = "evaporchain-mainnet";
        let (client, pm_addr, _shutdown) = spawn_paymaster_for_test(chain_id).await;

        // Wallet-side keypair (the user).
        let user_kp = HybridKeypair::generate();
        let sender: AccountAddress = *blake3::hash(&user_kp.public_key_bytes()).as_bytes();

        // Wallet reads paymaster info.
        let info = client.info().await.unwrap();
        assert_eq!(info.next_paymaster_nonce, 0);

        // Wallet builds the UserOp with paymaster + paymaster_nonce
        // pre-stamped (so the user-side signature commits to the
        // chosen paymaster).
        let inner = Transaction::Transfer(TransferTx {
            from: sender,
            to: [9u8; 32],
            amount: 500,
            nonce: 0,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        });
        let mut user_op = build_unsigned_user_op(sender, 0, &inner, 50_000).unwrap();
        user_op.paymaster = Some(pm_addr);
        user_op.paymaster_nonce = Some(info.next_paymaster_nonce);

        // 1. User signs the UserOp body.
        sign_user_op_as_sender(&mut user_op, chain_id, &user_kp);
        assert!(user_op.signature.is_some());
        assert!(user_op.public_key.is_some());

        // 2. POST to /sponsor — paymaster stamps its sig over the
        // sponsorship payload. The user-side fields are preserved.
        let resp = client.sponsor(user_op).await.unwrap();
        let returned = resp.user_op;
        // User's signature still present — paymaster did not touch it.
        assert!(returned.signature.is_some());
        assert!(returned.public_key.is_some());
        // Paymaster's sponsorship sig is now also present.
        assert!(returned.paymaster_signature.is_some());
        assert!(returned.paymaster_public_key.is_some());

        // 3a. User sig verifies against Transaction::UserOp.signing_message.
        let canonical_tx = Transaction::UserOp(returned.clone());
        let user_msg = canonical_tx.signing_message(chain_id);
        let user_sig = returned.signature.as_deref().unwrap();
        let user_pk = returned.public_key.as_deref().unwrap();
        assert!(
            HybridVerifier::verify(&user_msg, user_sig, user_pk),
            "user signature must verify under chain rules"
        );
        // Sender address must derive from user_pk (chain wires this in
        // verify_tx_signature via the Transaction's `from` field; for
        // UserOp the sender is committed in signable_bytes).
        let derived_sender: AccountAddress = *blake3::hash(user_pk).as_bytes();
        assert_eq!(derived_sender, sender);

        // 3b. Paymaster sig verifies against the sponsorship payload.
        let pm_payload = returned
            .paymaster_sponsorship_payload(chain_id)
            .expect("payload");
        let pm_sig = returned.paymaster_signature.as_deref().unwrap();
        let pm_pk = returned.paymaster_public_key.as_deref().unwrap();
        assert!(
            HybridVerifier::verify(&pm_payload, pm_sig, pm_pk),
            "paymaster signature must verify under chain rules"
        );
        let derived_pm: AccountAddress = *blake3::hash(pm_pk).as_bytes();
        assert_eq!(derived_pm, pm_addr);

        // Paymaster-stamped paymaster commitment matches what the user
        // signed over. Otherwise a malicious paymaster could redirect
        // the user's intent.
        assert_eq!(returned.paymaster, Some(pm_addr));
    }

    #[test]
    fn writes_keypair_in_paymaster_loadable_format() {
        // Sanity: the keypair generator the paymaster service ships with
        // produces a file that round-trips correctly. Catches any wallet-
        // side coupling (e.g. if the wallet reuses paymaster file format
        // for its own purposes).
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("paymaster_keypair.json");
        let original = generate_keypair_to_file(&path).expect("generate");
        let loaded =
            evaporchain_paymaster::load_keypair_from_file(&path).expect("load");
        // Functional equivalence: both keypairs sign+verify the same
        // canary.
        let msg = b"evaporchain-keypair-roundtrip-canary";
        let sig = original.sign(msg);
        assert!(HybridVerifier::verify(msg, &sig, &original.public_key_bytes()));
        let sig2 = loaded.sign(msg);
        assert!(HybridVerifier::verify(msg, &sig2, &loaded.public_key_bytes()));
    }
}
