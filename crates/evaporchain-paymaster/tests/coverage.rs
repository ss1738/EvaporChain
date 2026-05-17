//! Cross-component coverage for the paymaster sponsorship service.
//! Exercises sponsor / sponsor_idempotent / address derivation /
//! info / metrics / inner-variant whitelist / rate limit boundary
//! that the in-source unit tests don't span together.

use evaporchain_crypto::signatures::HybridKeypair;
use evaporchain_crypto::Signer;
use evaporchain_paymaster::{
    AuditFsyncMode, InnerVariant, Paymaster, PaymasterConfig, PaymasterError, SponsorOutcome,
    MAX_RATE_LIMIT_BUCKETS,
};
use evaporchain_types::{Transaction, TransferTx, UserOpTx};
use tempfile::TempDir;

const CHAIN_ID: &str = "evaporchain-cov-1";

fn nonce_path(dir: &TempDir) -> std::path::PathBuf {
    dir.path().join("paymaster_nonce")
}

fn permissive_paymaster(dir: &TempDir) -> Paymaster {
    Paymaster::new_with_config(
        HybridKeypair::generate(),
        CHAIN_ID,
        nonce_path(dir),
        PaymasterConfig::permissive(),
    )
    .unwrap()
}

fn empty_user_op(sender: [u8; 32]) -> UserOpTx {
    UserOpTx {
        sender,
        nonce: 0,
        call_data: vec![],
        call_gas_limit: 21_000,
        paymaster: None,
        paymaster_nonce: None,
        paymaster_data: None,
        paymaster_signature: None,
        paymaster_public_key: None,
        signature: None,
        public_key: None,
    }
}

// =================================================================
// Address derivation + chain_id
// =================================================================

#[test]
fn address_is_blake3_of_public_key() {
    let dir = TempDir::new().unwrap();
    let kp = HybridKeypair::generate();
    let expected = *blake3::hash(&kp.public_key_bytes()).as_bytes();
    let pm = Paymaster::new_with_config(
        kp,
        CHAIN_ID,
        nonce_path(&dir),
        PaymasterConfig::permissive(),
    )
    .unwrap();
    assert_eq!(pm.address(), expected);
    assert_eq!(pm.chain_id(), CHAIN_ID);
}

// =================================================================
// sponsor — happy path + already-signed
// =================================================================

#[test]
fn sponsor_assigns_monotonic_nonces() {
    let dir = TempDir::new().unwrap();
    let pm = permissive_paymaster(&dir);
    let mut op_a = empty_user_op([1u8; 32]);
    let mut op_b = empty_user_op([2u8; 32]);
    let n0 = pm.sponsor(&mut op_a).unwrap();
    let n1 = pm.sponsor(&mut op_b).unwrap();
    assert_eq!(n0, 0);
    assert_eq!(n1, 1);
    assert_eq!(op_a.paymaster_nonce, Some(0));
    assert_eq!(op_b.paymaster_nonce, Some(1));
    assert_eq!(op_a.paymaster, Some(pm.address()));
    assert_eq!(op_b.paymaster, Some(pm.address()));
    assert!(op_a.paymaster_signature.is_some());
    assert!(op_b.paymaster_signature.is_some());
    assert_eq!(pm.next_paymaster_nonce(), 2);
}

#[test]
fn sponsor_refuses_already_signed_user_op() {
    let dir = TempDir::new().unwrap();
    let pm = permissive_paymaster(&dir);
    let mut op = empty_user_op([3u8; 32]);
    op.paymaster_signature = Some(vec![0xAA; 96]);
    let err = pm.sponsor(&mut op).unwrap_err();
    assert!(matches!(err, PaymasterError::AlreadySigned));
    // Counter must NOT advance on rejection.
    assert_eq!(pm.next_paymaster_nonce(), 0);
}

// =================================================================
// Idempotency
// =================================================================

#[test]
fn sponsor_idempotent_no_key_is_always_fresh() {
    let dir = TempDir::new().unwrap();
    let pm = permissive_paymaster(&dir);
    let mut op = empty_user_op([7u8; 32]);
    let outcome = pm.sponsor_idempotent(None, &mut op).unwrap();
    assert!(matches!(outcome, SponsorOutcome::Fresh { .. }));
    assert_eq!(outcome.paymaster_nonce(), 0);
}

#[test]
fn sponsor_idempotent_same_key_replays_without_burning_nonce() {
    let dir = TempDir::new().unwrap();
    let pm = permissive_paymaster(&dir);
    let mut op_first = empty_user_op([7u8; 32]);
    let first = pm
        .sponsor_idempotent(Some("retry-1"), &mut op_first)
        .unwrap();
    assert!(matches!(first, SponsorOutcome::Fresh { paymaster_nonce: 0 }));

    // Second call under the same key must Replay — same nonce, same sig.
    let mut op_retry = empty_user_op([7u8; 32]);
    let second = pm
        .sponsor_idempotent(Some("retry-1"), &mut op_retry)
        .unwrap();
    match second {
        SponsorOutcome::Replay { paymaster_nonce } => assert_eq!(paymaster_nonce, 0),
        other => panic!("expected Replay, got {other:?}"),
    }
    assert_eq!(op_retry.paymaster_nonce, Some(0));
    assert_eq!(op_retry.paymaster_signature, op_first.paymaster_signature);
    // Next nonce stayed at 1 — no second allocation.
    assert_eq!(pm.next_paymaster_nonce(), 1);
}

#[test]
fn sponsor_idempotent_distinct_keys_get_distinct_nonces() {
    let dir = TempDir::new().unwrap();
    let pm = permissive_paymaster(&dir);
    let mut a = empty_user_op([1u8; 32]);
    let mut b = empty_user_op([2u8; 32]);
    let oa = pm.sponsor_idempotent(Some("k-a"), &mut a).unwrap();
    let ob = pm.sponsor_idempotent(Some("k-b"), &mut b).unwrap();
    assert!(matches!(oa, SponsorOutcome::Fresh { paymaster_nonce: 0 }));
    assert!(matches!(ob, SponsorOutcome::Fresh { paymaster_nonce: 1 }));
}

#[test]
fn sponsor_outcome_nonce_accessor() {
    let f = SponsorOutcome::Fresh { paymaster_nonce: 5 };
    let r = SponsorOutcome::Replay { paymaster_nonce: 9 };
    assert_eq!(f.paymaster_nonce(), 5);
    assert_eq!(r.paymaster_nonce(), 9);
}

// =================================================================
// PaymasterInfo / metrics
// =================================================================

#[test]
fn info_reflects_config_and_state() {
    let dir = TempDir::new().unwrap();
    let mut cfg = PaymasterConfig::permissive();
    cfg.allowed_inner_variants = Some(vec![InnerVariant::Transfer]);
    cfg.idempotency_max_keys = 64;
    cfg.idempotency_ttl_secs = 300;
    let pm = Paymaster::new_with_config(
        HybridKeypair::generate(),
        CHAIN_ID,
        nonce_path(&dir),
        cfg,
    )
    .unwrap();
    let info = pm.info();
    assert_eq!(info.chain_id, CHAIN_ID);
    assert_eq!(info.next_paymaster_nonce, 0);
    assert!(!info.require_user_sig);
    assert_eq!(info.idempotency_max_keys, 64);
    assert_eq!(info.idempotency_ttl_secs, 300);
    assert_eq!(
        info.allowed_inner_variants.as_deref(),
        Some(&["transfer".to_string()][..])
    );
    assert!(!info.audit_log_enabled);
    assert!(info.audit_log_fsync.is_none());
    let mut op = empty_user_op([1u8; 32]);
    pm.sponsor(&mut op).unwrap();
    assert_eq!(pm.info().next_paymaster_nonce, 1);
    // Metrics: 1 OK, 0 errors.
    let m = pm.metrics();
    assert_eq!(
        m.sponsorships_ok.load(std::sync::atomic::Ordering::Relaxed),
        1
    );
}

#[test]
fn prometheus_metrics_contains_canonical_lines() {
    let dir = TempDir::new().unwrap();
    let pm = permissive_paymaster(&dir);
    let mut op = empty_user_op([1u8; 32]);
    pm.sponsor(&mut op).unwrap();
    let text = pm.prometheus_metrics();
    assert!(text.contains("evaporchain_paymaster_sponsorships_total"));
    assert!(text.contains("evaporchain_paymaster_next_nonce 1"));
    assert!(text.contains("status=\"ok\""));
}

// =================================================================
// InnerVariant whitelist + CLI parsing
// =================================================================

#[test]
fn inner_variant_cli_round_trip() {
    for v in [
        InnerVariant::Transfer,
        InnerVariant::CallScript,
        InnerVariant::CallContract,
    ] {
        let s = v.as_str();
        assert_eq!(InnerVariant::parse_cli(s), Some(v));
    }
    assert!(InnerVariant::parse_cli("bogus").is_none());
    assert!(InnerVariant::parse_cli("Transfer").is_none()); // case-sensitive
}

#[test]
fn inner_variant_from_transaction_tags_supported_variants() {
    let tx = Transaction::Transfer(TransferTx {
        from: [0u8; 32],
        to: [1u8; 32],
        amount: 1,
        nonce: 0,
        signature: None,
        public_key: None,
        mev_refund_eligible: None,
    });
    assert_eq!(InnerVariant::from_transaction(&tx), Some(InnerVariant::Transfer));
}

#[test]
fn sponsor_rejects_inner_variant_not_in_whitelist() {
    let dir = TempDir::new().unwrap();
    let mut cfg = PaymasterConfig::permissive();
    cfg.allowed_inner_variants = Some(vec![InnerVariant::CallScript]); // Transfer NOT allowed
    let pm = Paymaster::new_with_config(
        HybridKeypair::generate(),
        CHAIN_ID,
        nonce_path(&dir),
        cfg,
    )
    .unwrap();

    let inner = Transaction::Transfer(TransferTx {
        from: [0u8; 32],
        to: [1u8; 32],
        amount: 1,
        nonce: 0,
        signature: None,
        public_key: None,
        mev_refund_eligible: None,
    });
    let mut op = empty_user_op([1u8; 32]);
    op.call_data = serde_json::to_vec(&inner).unwrap();

    let err = pm.sponsor(&mut op).unwrap_err();
    match err {
        PaymasterError::InnerVariantNotAllowed { variant } => {
            assert_eq!(variant, "transfer");
        }
        other => panic!("expected InnerVariantNotAllowed, got {other:?}"),
    }
    // Nonce did not advance on rejection.
    assert_eq!(pm.next_paymaster_nonce(), 0);
}

#[test]
fn sponsor_allows_inner_variant_in_whitelist() {
    let dir = TempDir::new().unwrap();
    let mut cfg = PaymasterConfig::permissive();
    cfg.allowed_inner_variants = Some(vec![InnerVariant::Transfer]);
    let pm = Paymaster::new_with_config(
        HybridKeypair::generate(),
        CHAIN_ID,
        nonce_path(&dir),
        cfg,
    )
    .unwrap();
    let inner = Transaction::Transfer(TransferTx {
        from: [0u8; 32],
        to: [1u8; 32],
        amount: 1,
        nonce: 0,
        signature: None,
        public_key: None,
        mev_refund_eligible: None,
    });
    let mut op = empty_user_op([1u8; 32]);
    op.call_data = serde_json::to_vec(&inner).unwrap();
    pm.sponsor(&mut op).unwrap();
    assert_eq!(op.paymaster_nonce, Some(0));
}

#[test]
fn sponsor_empty_call_data_bypasses_inner_whitelist() {
    let dir = TempDir::new().unwrap();
    let mut cfg = PaymasterConfig::permissive();
    cfg.allowed_inner_variants = Some(vec![InnerVariant::CallScript]); // Transfer NOT allowed
    let pm = Paymaster::new_with_config(
        HybridKeypair::generate(),
        CHAIN_ID,
        nonce_path(&dir),
        cfg,
    )
    .unwrap();
    // call_data is empty (gas-only sponsorship) → always allowed.
    let mut op = empty_user_op([1u8; 32]);
    assert!(op.call_data.is_empty());
    pm.sponsor(&mut op).unwrap();
    assert_eq!(op.paymaster_nonce, Some(0));
}

// =================================================================
// Defaults + permissive config + constants
// =================================================================

#[test]
fn paymaster_config_default_is_strict() {
    let d = PaymasterConfig::default();
    assert!(d.require_user_sig);
    assert!(d.per_sender_rps > 0.0);
    assert!(d.per_sender_burst > 0);
    assert!(matches!(d.audit_log_fsync, AuditFsyncMode::PerLine));
    assert!(d.audit_log.is_none());
    assert!(d.allowed_inner_variants.is_none());
}

#[test]
fn paymaster_config_permissive_disables_user_sig_and_rate_limit() {
    let p = PaymasterConfig::permissive();
    assert!(!p.require_user_sig);
    assert_eq!(p.per_sender_rps, 0.0);
    assert_eq!(p.per_sender_burst, 0);
    // Idempotency stays enabled even in permissive mode.
    assert!(p.idempotency_max_keys > 0);
}

#[test]
fn rate_limit_bucket_cap_is_pinned() {
    assert_eq!(MAX_RATE_LIMIT_BUCKETS, 1usize << 20);
}

// =================================================================
// Nonce persistence across paymaster restarts
// =================================================================

#[test]
fn nonce_persists_across_paymaster_restarts() {
    let dir = TempDir::new().unwrap();
    let path = nonce_path(&dir);
    let kp_bytes;
    {
        let kp = HybridKeypair::generate();
        kp_bytes = kp.public_key_bytes();
        let pm = Paymaster::new_with_config(
            kp,
            CHAIN_ID,
            path.clone(),
            PaymasterConfig::permissive(),
        )
        .unwrap();
        for i in 0..3 {
            let mut op = empty_user_op([i as u8; 32]);
            pm.sponsor(&mut op).unwrap();
        }
        assert_eq!(pm.next_paymaster_nonce(), 3);
    }
    // Reopen with a different keypair — nonce file is per-instance, so
    // the new paymaster picks up from 3 regardless of identity.
    let pm2 = Paymaster::new_with_config(
        HybridKeypair::generate(),
        CHAIN_ID,
        path,
        PaymasterConfig::permissive(),
    )
    .unwrap();
    assert_eq!(pm2.next_paymaster_nonce(), 3);
    // Different keypair → different address.
    let new_addr = *blake3::hash(&pm2.address()).as_bytes();
    assert_ne!(new_addr, *blake3::hash(&kp_bytes).as_bytes());
}
