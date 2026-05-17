//! Day 4 E2E — paymaster service through full block execution.
//!
//! Closes the chain-rule binding under real `execute_block`, not just
//! the private `execute_user_op` path (which the Day 1 tests already
//! cover). Pipeline exercised:
//!
//!   wallet  →  POST /sponsor (real axum task)  →  paymaster.sponsor
//!           →  signed UserOpTx  →  wrap in Block
//!           →  SimpleExecutor::execute_block (full dispatch loop)
//!           →  state mutations applied
//!
//! Verifies: sender debited by inner-Transfer amount, paymaster gas
//! debited (call_gas_limit + GAS_USER_OP), recipient credited,
//! sender's nonce bumped exactly once.

#![cfg(test)]

use std::sync::Arc;

use evaporchain_execution::{ExecutionEngine, SimpleExecutor};
use evaporchain_paymaster::{
    Paymaster, PaymasterInfo, SponsorshipRequest, SponsorshipResponse,
};
use evaporchain_state::db::StateDB;
use evaporchain_state::InMemoryStateDB;
use evaporchain_types::{
    AccountAddress, Account, Block, Transaction, TransferTx, UserOpTx,
};

use evaporchain_crypto::signatures::HybridKeypair;

/// Spawn the real paymaster HTTP server in a tokio task with the
/// permissive profile (no user-sig pre-check, no rate limiting). Used
/// by tests that exercise chain-side enforcement and don't construct
/// user-side signatures.
async fn spawn_paymaster(
    chain_id: &str,
) -> (
    String, /* base url */
    AccountAddress,
    tokio::sync::oneshot::Sender<()>,
) {
    spawn_paymaster_with_config(
        chain_id,
        evaporchain_paymaster::PaymasterConfig::permissive(),
    )
    .await
}

/// Spawn the real paymaster HTTP server with explicit config. The
/// strict-mode E2E test (audit fix #7) uses this with the default
/// (`require_user_sig: true`) config so the full
/// chain-id-bound-user-sig + sponsorship-sig + execute_block
/// pipeline is proven end-to-end in a single test.
async fn spawn_paymaster_with_config(
    chain_id: &str,
    config: evaporchain_paymaster::PaymasterConfig,
) -> (
    String, /* base url */
    AccountAddress,
    tokio::sync::oneshot::Sender<()>,
) {
    use axum::{
        extract::State, http::StatusCode, routing::{get, post}, Json, Router,
    };

    let tmp = tempfile::TempDir::new().unwrap();
    let nonce_file = tmp.path().join("paymaster_nonce");
    Box::leak(Box::new(tmp));

    let kp = HybridKeypair::generate();
    let paymaster = Arc::new(
        Paymaster::new_with_config(kp, chain_id.to_string(), nonce_file, config)
            .expect("paymaster"),
    );
    let pm_addr = paymaster.address();

    #[derive(Clone)]
    struct AppState {
        paymaster: Arc<Paymaster>,
    }
    async fn get_info(State(s): State<AppState>) -> Json<PaymasterInfo> {
        Json(s.paymaster.info())
    }
    async fn sponsor(
        State(s): State<AppState>,
        headers: axum::http::HeaderMap,
        Json(req): Json<SponsorshipRequest>,
    ) -> Result<Json<SponsorshipResponse>, (StatusCode, String)> {
        // Mirror the binary's Day 12 idempotency wiring.
        let idempotency_key = headers
            .get("idempotency-key")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let mut user_op = req.user_op;
        let outcome = s
            .paymaster
            .sponsor_idempotent(idempotency_key.as_deref(), &mut user_op)
            .map_err(|e: evaporchain_paymaster::PaymasterError| {
                (StatusCode::BAD_REQUEST, e.to_string())
            })?;
        let assigned = outcome.paymaster_nonce();
        Ok(Json(SponsorshipResponse {
            user_op,
            paymaster_address_hex: hex::encode(s.paymaster.address()),
            paymaster_nonce: assigned,
        }))
    }

    let app = Router::new()
        .route("/info", get(get_info))
        .route("/sponsor", post(sponsor))
        .with_state(AppState { paymaster });

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            })
            .await
            .ok();
    });
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    (format!("http://{addr}"), pm_addr, shutdown_tx)
}

fn block_with_one_tx(number: u64, tx: Transaction) -> Block {
    Block {
        number,
        epoch: 0,
        parent_hash: [0u8; 32],
        state_root: [0u8; 32],
        transactions: vec![tx],
        timestamp: 1_700_000_000,
        chain_id: String::new(),
        producer_id: Some(0),
        vrf_output: None,
        vrf_proof: None,
        data_root: None,
        da_row_roots: vec![],
        da_col_roots: vec![],
        blob_commitments: vec![],
        da_certificate: None,
        commit_certificate: None,
        nova_proof: None,
        anchor_hash: None,
        state_function_commitment: None,
        oracle_state_root: None,
        shard_count: None,
        protocol_version: 0,
        state_root_version: 0,
        submit_epoch_hints: vec![],
        parents: vec![],
        post_state_root: None,
    }
}

#[tokio::test]
async fn paymaster_e2e_sponsored_transfer_through_full_block_execution() {
    // Chain id "" matches `SimpleExecutor::new()`'s default chain_id +
    // an empty `Block::chain_id`. The paymaster signs sponsorship under
    // chain_id "", and execute_user_op verifies under self.chain_id "".
    let chain_id = "";
    let (pm_url, pm_addr, _shutdown) = spawn_paymaster(chain_id).await;

    // ── 1. Wallet builds a Transfer intent + half-formed UserOp ──────
    let sender: AccountAddress = [1u8; 32];
    let recipient: AccountAddress = [9u8; 32];
    let inner = Transaction::Transfer(TransferTx {
        from: sender,
        to: recipient,
        amount: 500,
        nonce: 0,
        signature: None,
        public_key: None,
        mev_refund_eligible: None,
    });
    let user_op = UserOpTx {
        sender,
        nonce: 0,
        call_data: serde_json::to_vec(&inner).unwrap(),
        call_gas_limit: 50_000,
        paymaster: None,
        paymaster_nonce: None,
        paymaster_data: None,
        paymaster_signature: None,
        paymaster_public_key: None,
        signature: None,
        public_key: None,
    };

    // ── 2. Wallet POSTs to /sponsor ──────────────────────────────────
    let http = reqwest::Client::new();
    let req = SponsorshipRequest { user_op };
    let resp: SponsorshipResponse = http
        .post(format!("{pm_url}/sponsor"))
        .json(&req)
        .send()
        .await
        .expect("POST /sponsor")
        .error_for_status()
        .expect("2xx")
        .json()
        .await
        .expect("decode response");
    assert_eq!(resp.paymaster_address_hex, hex::encode(pm_addr));
    assert_eq!(resp.paymaster_nonce, 0);
    let signed_user_op = resp.user_op;

    // ── 3. Wrap signed UserOp in a Block, run through execute_block ──
    let mut db = InMemoryStateDB::new();
    db.put_account(Account {
        address: sender,
        balance: 1_000,
        nonce: 0,
        storage_deposit: 0,
        storage_bytes: 0,
        last_touched_epoch: 0,
        vesting: None,
    });
    db.put_account(Account {
        address: pm_addr,
        balance: 1_000_000,
        nonce: 0,
        storage_deposit: 0,
        storage_bytes: 0,
        last_touched_epoch: 0,
        vesting: None,
    });

    let mut executor = SimpleExecutor::new(7);
    let block = block_with_one_tx(1, Transaction::UserOp(signed_user_op));
    let result = executor
        .execute_block(&mut db, &block)
        .expect("execute_block");
    assert_eq!(
        result.txs_failed, 0,
        "no tx should fail; outcomes={:?}",
        result.tx_outcomes
    );
    assert_eq!(result.txs_executed, 1);

    // ── 4. Assert state mutations ────────────────────────────────────
    // Sender debited by 500 (inner Transfer amount).
    let s = db.get_account(&sender).expect("sender exists");
    assert_eq!(s.balance, 500, "sender debited by inner transfer");
    // Sender's nonce bumped exactly once (outer UserOp consumes one).
    assert_eq!(s.nonce, 1, "sender nonce bumped exactly once, not twice");

    // Recipient credited 500.
    let r = db.get_account(&recipient).expect("recipient exists");
    assert_eq!(r.balance, 500);

    // Paymaster paid gas; balance below original.
    let pm = db.get_account(&pm_addr).expect("paymaster exists");
    assert!(
        pm.balance < 1_000_000,
        "paymaster gas-debited; balance now {}",
        pm.balance
    );
    // Paymaster's nonce bumped (sponsorship counter advanced).
    assert_eq!(pm.nonce, 1);
}

#[tokio::test]
async fn paymaster_e2e_rejects_tampered_call_data_at_chain_layer() {
    // After getting a signed UserOp, an attacker tampers call_data
    // before submission. The chain's HybridVerifier::verify must
    // reject because the sponsorship payload binds blake3(call_data).
    // execute_block returns the failure in tx_outcomes — paymaster is
    // not debited, sender is not debited.
    let chain_id = "";
    let (pm_url, pm_addr, _shutdown) = spawn_paymaster(chain_id).await;

    let sender: AccountAddress = [1u8; 32];
    let original = Transaction::Transfer(TransferTx {
        from: sender,
        to: [9u8; 32],
        amount: 500,
        nonce: 0,
        signature: None,
        public_key: None,
        mev_refund_eligible: None,
    });
    let user_op = UserOpTx {
        sender,
        nonce: 0,
        call_data: serde_json::to_vec(&original).unwrap(),
        call_gas_limit: 50_000,
        paymaster: None,
        paymaster_nonce: None,
        paymaster_data: None,
        paymaster_signature: None,
        paymaster_public_key: None,
        signature: None,
        public_key: None,
    };

    let http = reqwest::Client::new();
    let req = SponsorshipRequest { user_op };
    let resp: SponsorshipResponse = http
        .post(format!("{pm_url}/sponsor"))
        .json(&req)
        .send()
        .await
        .expect("POST")
        .json()
        .await
        .expect("json");
    let mut signed_user_op = resp.user_op;

    // Tamper: swap the inner transfer's amount to 1_000_000 (above
    // sender's funded balance, but the real defense is the sig).
    let tampered = Transaction::Transfer(TransferTx {
        from: sender,
        to: [9u8; 32],
        amount: 1_000_000,
        nonce: 0,
        signature: None,
        public_key: None,
        mev_refund_eligible: None,
    });
    signed_user_op.call_data = serde_json::to_vec(&tampered).unwrap();

    let mut db = InMemoryStateDB::new();
    db.put_account(Account {
        address: sender,
        balance: 1_000,
        nonce: 0,
        storage_deposit: 0,
        storage_bytes: 0,
        last_touched_epoch: 0,
        vesting: None,
    });
    db.put_account(Account {
        address: pm_addr,
        balance: 1_000_000,
        nonce: 0,
        storage_deposit: 0,
        storage_bytes: 0,
        last_touched_epoch: 0,
        vesting: None,
    });

    let mut executor = SimpleExecutor::new(7);
    let block = block_with_one_tx(1, Transaction::UserOp(signed_user_op));
    let result = executor
        .execute_block(&mut db, &block)
        .expect("execute_block returns Ok with per-tx outcomes");

    assert_eq!(result.txs_failed, 1);
    assert_eq!(result.txs_executed, 0);
    let outcome = &result.tx_outcomes[0];
    assert!(!outcome.success);
    let err_msg = outcome.error.as_deref().unwrap_or("");
    assert!(
        err_msg.contains("verification failed") || err_msg.contains("paymaster_signature"),
        "expected sponsorship sig rejection, got: {err_msg}"
    );
    // Defense-in-depth: paymaster + sender state untouched.
    let pm = db.get_account(&pm_addr).expect("paymaster exists");
    assert_eq!(pm.balance, 1_000_000, "paymaster NOT debited on tamper");
    assert_eq!(pm.nonce, 0);
    let s = db.get_account(&sender).expect("sender exists");
    assert_eq!(s.balance, 1_000, "sender NOT debited on tamper");
}

// ─── Audit fix #7 (2026-05-09): strict-mode E2E ────────────────────────
//
// Pre-fix the paymaster_e2e suite ran exclusively under
// `PaymasterConfig::permissive()` (no user-sig pre-check). The full
// chain-id-bound user-sig + sponsorship-sig + execute_block pipeline
// worked by construction — both ends use the same canonical messages
// — but no test exercised it together.
//
// This test closes that gap. Pipeline:
//   1. Chain runs with verify_signatures: true, chain_id bound.
//   2. Wallet (user) signs `Transaction::UserOp(user_op).signing_message(chain_id)`.
//   3. POST /sponsor — paymaster verifies the user sig (strict mode)
//      and only then signs sponsorship.
//   4. Returned UserOp wrapped in a Block.
//   5. SimpleExecutor::new_with_sig_verification verifies the user
//      sig AGAIN at execute time (chain-side); execute_user_op
//      verifies the sponsorship sig.
//   6. Inner Transfer dispatched — sender debited, recipient credited,
//      paymaster gas debited.
//
// Two invariants asserted: txs_executed == 1, AND every sig check
// along the way passed (otherwise the tx would land as txs_failed).

#[tokio::test]
async fn paymaster_e2e_strict_mode_full_pipeline() {
    use evaporchain_crypto::signatures::{HybridKeypair, Signer};

    let chain_id = "evaporchain-strict-e2e";
    let (pm_url, pm_addr, _shutdown) = spawn_paymaster_with_config(
        chain_id,
        evaporchain_paymaster::PaymasterConfig::default(),
    )
    .await;

    // ── Wallet side: build + sign UserOp.
    let user_kp = HybridKeypair::generate();
    // address = blake3(ADDRESS_DST || pk) — must match address_from_pubkey in executor I1 check.
    let sender: AccountAddress = evaporchain_types::address_from_pubkey(&user_kp.public_key_bytes());
    let recipient: AccountAddress = [9u8; 32];

    let inner = Transaction::Transfer(TransferTx {
        from: sender,
        to: recipient,
        amount: 500,
        nonce: 0,
        signature: None,
        public_key: None,
        mev_refund_eligible: None,
    });
    let mut user_op = UserOpTx {
        sender,
        nonce: 0,
        call_data: serde_json::to_vec(&inner).unwrap(),
        call_gas_limit: 50_000,
        // Wallet pre-stamps paymaster + paymaster_nonce so the user sig
        // commits to BOTH (the paymaster overwrites paymaster to its
        // own address before the strict-mode user-sig check; if the
        // wallet pre-stamps a different paymaster, the user sig would
        // mismatch and /sponsor would 400).
        paymaster: Some(pm_addr),
        paymaster_nonce: Some(0),
        paymaster_data: None,
        paymaster_signature: None,
        paymaster_public_key: None,
        signature: None,
        public_key: None,
    };
    // User signs over the chain-canonical message (same shape both
    // paymaster strict-mode AND chain verify_tx_signature use).
    let canonical = Transaction::UserOp(user_op.clone()).signing_message(chain_id);
    user_op.signature = Some(user_kp.sign(&canonical));
    user_op.public_key = Some(user_kp.public_key_bytes());

    // ── POST /sponsor — paymaster verifies user sig + signs sponsorship.
    let http = reqwest::Client::new();
    let req = SponsorshipRequest { user_op };
    let resp: SponsorshipResponse = http
        .post(format!("{pm_url}/sponsor"))
        .json(&req)
        .send()
        .await
        .expect("POST")
        .error_for_status()
        .expect("strict-mode /sponsor must succeed for properly signed UserOp")
        .json()
        .await
        .expect("decode");
    let signed_user_op = resp.user_op;
    assert!(signed_user_op.signature.is_some(), "user sig preserved");
    assert!(signed_user_op.paymaster_signature.is_some(), "sponsorship sig stamped");

    // ── Chain side: full verify_signatures=true block execution.
    let mut db = InMemoryStateDB::new();
    db.put_account(Account {
        address: sender,
        balance: 1_000,
        nonce: 0,
        storage_deposit: 0,
        storage_bytes: 0,
        last_touched_epoch: 0,
        vesting: None,
    });
    db.put_account(Account {
        address: pm_addr,
        balance: 1_000_000,
        nonce: 0,
        storage_deposit: 0,
        storage_bytes: 0,
        last_touched_epoch: 0,
        vesting: None,
    });

    let mut executor = evaporchain_execution::SimpleExecutor::new_with_sig_verification(7);
    executor.set_chain_id(chain_id.to_string());

    let mut block = block_with_one_tx(1, Transaction::UserOp(signed_user_op));
    block.chain_id = chain_id.to_string();
    let result = executor
        .execute_block(&mut db, &block)
        .expect("execute_block returns Ok with per-tx outcomes");
    assert_eq!(
        result.txs_failed, 0,
        "strict-mode E2E must pass every sig check; outcomes={:?}",
        result.tx_outcomes
    );
    assert_eq!(result.txs_executed, 1);

    // ── State assertions.
    let s = db.get_account(&sender).expect("sender");
    assert_eq!(s.balance, 500, "sender debited inner transfer");
    assert_eq!(s.nonce, 1, "sender nonce bumped exactly once");
    let r = db.get_account(&recipient).expect("recipient");
    assert_eq!(r.balance, 500);
    let pm = db.get_account(&pm_addr).expect("paymaster");
    assert!(pm.balance < 1_000_000, "paymaster gas-debited");
    assert_eq!(pm.nonce, 1, "paymaster nonce bumped");
}

#[tokio::test]
async fn paymaster_e2e_strict_mode_rejects_unsigned_userop() {
    // Negative case: strict-mode paymaster receives a UserOp without
    // a user signature. /sponsor returns 400 InvalidUserSignature
    // BEFORE allocating a sponsorship nonce — the paymaster pays no
    // gas and the chain never sees the bad request.
    let chain_id = "evaporchain-strict-e2e-neg";
    let (pm_url, pm_addr, _shutdown) = spawn_paymaster_with_config(
        chain_id,
        evaporchain_paymaster::PaymasterConfig::default(),
    )
    .await;

    let inner = Transaction::Transfer(TransferTx {
        from: [1u8; 32],
        to: [9u8; 32],
        amount: 1,
        nonce: 0,
        signature: None,
        public_key: None,
        mev_refund_eligible: None,
    });
    let user_op = UserOpTx {
        sender: [1u8; 32],
        nonce: 0,
        call_data: serde_json::to_vec(&inner).unwrap(),
        call_gas_limit: 1_000,
        paymaster: Some(pm_addr),
        paymaster_nonce: Some(0),
        paymaster_data: None,
        paymaster_signature: None,
        paymaster_public_key: None,
        signature: None, // ← no user sig
        public_key: None,
    };

    let http = reqwest::Client::new();
    let resp = http
        .post(format!("{pm_url}/sponsor"))
        .json(&SponsorshipRequest { user_op })
        .send()
        .await
        .expect("POST");
    assert_eq!(
        resp.status().as_u16(),
        400,
        "strict-mode /sponsor must reject UserOp without user sig"
    );
    let body = resp.text().await.unwrap_or_default();
    assert!(
        body.contains("user signature missing or invalid"),
        "error body must name the user sig as the cause: got {body:?}"
    );
}
