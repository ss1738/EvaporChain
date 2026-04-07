//! Behavior tests for offline signing and transaction export.
//!
//! Validates the air-gapped signing workflow: build → sign → export → load → verify.

use evaporchain_crypto::signatures::{MlDsaKeypair, MlDsaVerifier, Signer, Verifier};
use evaporchain_wallet::offline::{OfflineSigner, SignedTransaction};
use evaporchain_wallet::signer::WalletSigner;
use evaporchain_wallet::address::format_address;
use std::path::PathBuf;

fn make_signer() -> WalletSigner {
    WalletSigner::from_keypair(MlDsaKeypair::generate())
}

fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir()
        .join(format!("evaporchain_offline_behavior_{}_{}", std::process::id(), name))
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 1: Offline transfer — sign, export to file, load, verify
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn offline_transfer_full_workflow() {
    let signer = make_signer();
    let to = [0xBBu8; 32];

    // Step 1: Sign offline
    let signed = OfflineSigner::sign_transfer(&signer, &to, 5000, 7);
    assert_eq!(signed.tx_type, "Transfer");
    assert_eq!(signed.from, format_address(signer.address()));
    assert_eq!(signed.to.as_deref(), Some(format_address(&to).as_str()));
    assert_eq!(signed.amount, Some(5000));
    assert_eq!(signed.nonce, 7);
    assert!(!signed.signature.is_empty());
    assert!(!signed.public_key.is_empty());
    assert!(!signed.signed_at.is_empty());

    // Step 2: Export to file
    let path = temp_path("transfer.json");
    signed.save(&path).unwrap();

    // Step 3: Load from file (on a different machine)
    let loaded = SignedTransaction::load(&path).unwrap();
    assert_eq!(loaded.tx_type, signed.tx_type);
    assert_eq!(loaded.from, signed.from);
    assert_eq!(loaded.to, signed.to);
    assert_eq!(loaded.amount, signed.amount);
    assert_eq!(loaded.nonce, signed.nonce);
    assert_eq!(loaded.signature, signed.signature);
    assert_eq!(loaded.public_key, signed.public_key);

    let _ = std::fs::remove_file(&path);
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 2: Offline refresh transaction
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn offline_refresh_sign_and_export() {
    let signer = make_signer();
    let obj_id = [0xCCu8; 32];

    let signed = OfflineSigner::sign_refresh(&signer, &obj_id, 1500);
    assert_eq!(signed.tx_type, "Refresh");
    assert!(signed.extra.is_some());

    let extra = signed.extra.as_ref().unwrap();
    assert_eq!(extra["energy"], 1500);

    // JSON roundtrip
    let json = serde_json::to_string(&signed).unwrap();
    let loaded: SignedTransaction = serde_json::from_str(&json).unwrap();
    assert_eq!(loaded.tx_type, "Refresh");
    assert_eq!(loaded.signature, signed.signature);
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 3: Offline create-object transaction
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn offline_create_object_sign_and_export() {
    let signer = make_signer();
    let obj_id = [0xDDu8; 32];

    let signed = OfflineSigner::sign_create_object(
        &signer,
        &obj_id,
        10000,
        200,
        vec![0xAB; 64],
    );
    assert_eq!(signed.tx_type, "CreateObject");
    assert!(signed.extra.is_some());

    let extra = signed.extra.as_ref().unwrap();
    assert_eq!(extra["energy"], 10000);
    assert_eq!(extra["half_life"], 200);

    // File roundtrip
    let path = temp_path("create_object.json");
    signed.save(&path).unwrap();
    let loaded = SignedTransaction::load(&path).unwrap();
    assert_eq!(loaded.tx_type, "CreateObject");
    assert_eq!(loaded.extra, signed.extra);

    let _ = std::fs::remove_file(&path);
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 4: Different signers produce different offline signatures
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn different_signers_different_offline_signatures() {
    let s1 = make_signer();
    let s2 = make_signer();
    let to = [1u8; 32];

    let sig1 = OfflineSigner::sign_transfer(&s1, &to, 1000, 0);
    let sig2 = OfflineSigner::sign_transfer(&s2, &to, 1000, 0);

    assert_ne!(sig1.signature, sig2.signature);
    assert_ne!(sig1.public_key, sig2.public_key);
    assert_ne!(sig1.from, sig2.from);
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 5: Signed transaction JSON format correctness
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn signed_transaction_json_format() {
    let signer = make_signer();
    let to = [0xBBu8; 32];
    let signed = OfflineSigner::sign_transfer(&signer, &to, 1000, 0);

    let json = serde_json::to_string_pretty(&signed).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

    // Required fields present
    assert!(parsed["tx_type"].is_string());
    assert!(parsed["from"].is_string());
    assert!(parsed["nonce"].is_number());
    assert!(parsed["signature"].is_string());
    assert!(parsed["public_key"].is_string());
    assert!(parsed["signed_at"].is_string());

    // Transfer-specific fields
    assert!(parsed["to"].is_string());
    assert!(parsed["amount"].is_number());

    // Addresses are hex-formatted
    assert!(parsed["from"].as_str().unwrap().starts_with("0x"));
    assert!(parsed["to"].as_str().unwrap().starts_with("0x"));

    // No extra field for transfer
    assert!(parsed.get("extra").is_none() || parsed["extra"].is_null());
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 6: Multiple transactions batch offline signing
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn batch_offline_signing() {
    let signer = make_signer();

    // Sign a batch of 10 transfers
    let mut signed_txs = Vec::new();
    for i in 0..10u64 {
        let mut to = [0u8; 32];
        to[0] = (i + 1) as u8;
        let signed = OfflineSigner::sign_transfer(&signer, &to, (i + 1) * 100, i);
        signed_txs.push(signed);
    }

    // All have unique nonces and amounts
    for (i, tx) in signed_txs.iter().enumerate() {
        assert_eq!(tx.nonce, i as u64);
        assert_eq!(tx.amount, Some((i as u64 + 1) * 100));
    }

    // All signatures are different (different messages)
    for i in 0..10 {
        for j in (i + 1)..10 {
            assert_ne!(
                signed_txs[i].signature, signed_txs[j].signature,
                "txs {} and {} should have different signatures",
                i, j
            );
        }
    }

    // All can be exported and loaded
    let dir = temp_path("batch");
    std::fs::create_dir_all(&dir).unwrap();
    for (i, tx) in signed_txs.iter().enumerate() {
        let path = dir.join(format!("tx_{}.json", i));
        tx.save(&path).unwrap();
        let loaded = SignedTransaction::load(&path).unwrap();
        assert_eq!(loaded.signature, tx.signature);
    }

    let _ = std::fs::remove_dir_all(&dir);
}
