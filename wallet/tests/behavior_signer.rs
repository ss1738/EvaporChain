//! Behavior tests for the EvaporChain wallet signer.
//!
//! Verifies ML-DSA signing and verification across all 9 transaction types,
//! keystore-backed signing, and signature integrity properties.

use evaporchain_crypto::signatures::{MlDsaKeypair, MlDsaVerifier, Verifier};
use evaporchain_wallet::address::derive_address;
use evaporchain_wallet::keystore::KeyStore;
use evaporchain_wallet::signer::WalletSigner;
use evaporchain_wallet::tx_builder::TxBuilder;

fn make_signer() -> WalletSigner {
    WalletSigner::from_keypair(MlDsaKeypair::generate())
}

fn make_two_signers() -> (WalletSigner, WalletSigner) {
    (make_signer(), make_signer())
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 1: Sign and verify all 9 transaction types
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn sign_and_verify_all_transaction_types() {
    let signer = make_signer();
    let addr = *signer.address();
    let builder = TxBuilder::new(addr);
    let recipient = [0xBBu8; 32];
    let obj_id = [0xCCu8; 32];

    let transactions = vec![
        ("Transfer", builder.transfer(recipient, 1000, 0)),
        (
            "CreateObject",
            builder.create_object(obj_id, 5000, 100, vec![0xAB; 16]),
        ),
        ("Refresh", builder.refresh(obj_id, 500)),
        (
            "DeployContract",
            builder.deploy_contract("Token", "{}", 10000, 200),
        ),
        (
            "CallContract",
            builder.call_contract(1, "transfer", "{}", 42),
        ),
        (
            "DeployScript",
            builder.deploy_script("fn main() {}", 8000, 150),
        ),
        ("CallScript", builder.call_script(1, "run", "[]", 10)),
        (
            "ValidatorStake",
            builder.validator_stake(1, 100_000, 0, None),
        ),
        ("ValidatorExit", builder.validator_exit(1, 3)),
    ];

    for (name, tx) in transactions {
        // All start unsigned
        assert!(tx.signature().is_none(), "{} should start unsigned", name);
        assert!(
            tx.public_key().is_none(),
            "{} should have no pubkey initially",
            name
        );

        // Sign (chain-id bound — the only form the chain accepts)
        let signed = signer.sign_for_chain(&tx, "evaporchain-testnet");

        // Signature and pubkey are set
        let sig = signed
            .signature()
            .unwrap_or_else(|| panic!("{} missing signature", name));
        let pk = signed
            .public_key()
            .unwrap_or_else(|| panic!("{} missing pubkey", name));
        assert!(!sig.is_empty(), "{} signature empty", name);
        assert!(!pk.is_empty(), "{} pubkey empty", name);

        // Verify signature against the chain-id-bound signing message
        let msg = signed.signing_message("evaporchain-testnet");
        assert!(
            MlDsaVerifier::verify(&msg, sig, pk),
            "{} signature verification failed",
            name
        );

        // Original remains unsigned
        assert!(tx.signature().is_none(), "{} original was mutated", name);
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 2: sign_transaction mutates in-place
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn sign_transaction_mutates_in_place() {
    let signer = make_signer();
    let builder = TxBuilder::new(*signer.address());
    let mut tx = builder.transfer([1u8; 32], 500, 1);

    assert!(tx.signature().is_none());
    signer.sign_transaction_for_chain(&mut tx, "evaporchain-testnet");
    assert!(tx.signature().is_some());

    // Verify the in-place signature against the chain-id-bound signing message
    let msg = tx.signing_message("evaporchain-testnet");
    let sig = tx.signature().unwrap();
    let pk = tx.public_key().unwrap();
    assert!(MlDsaVerifier::verify(&msg, sig, pk));
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 3: Different signers produce different signatures
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn different_signers_different_signatures() {
    let (s1, s2) = make_two_signers();

    // Same transaction content but different senders
    let tx1 = TxBuilder::new(*s1.address()).transfer([1u8; 32], 1000, 0);
    let tx2 = TxBuilder::new(*s2.address()).transfer([1u8; 32], 1000, 0);

    let signed1 = s1.sign_for_chain(&tx1, "evaporchain-testnet");
    let signed2 = s2.sign_for_chain(&tx2, "evaporchain-testnet");

    // Different signatures
    assert_ne!(signed1.signature().unwrap(), signed2.signature().unwrap());

    // Different public keys
    assert_ne!(signed1.public_key().unwrap(), signed2.public_key().unwrap());

    // Both verify with their own keys
    let msg1 = signed1.signing_message("evaporchain-testnet");
    let msg2 = signed2.signing_message("evaporchain-testnet");
    assert!(MlDsaVerifier::verify(
        &msg1,
        signed1.signature().unwrap(),
        signed1.public_key().unwrap()
    ));
    assert!(MlDsaVerifier::verify(
        &msg2,
        signed2.signature().unwrap(),
        signed2.public_key().unwrap()
    ));
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 4: Cross-signer verification fails
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn cross_signer_verification_fails() {
    let (s1, s2) = make_two_signers();
    let builder = TxBuilder::new(*s1.address());
    let tx = builder.transfer([1u8; 32], 1000, 0);
    let signed = s1.sign_for_chain(&tx, "evaporchain-testnet");

    let msg = signed.signing_message("evaporchain-testnet");
    let sig = signed.signature().unwrap();

    // Verify with s1's key passes
    assert!(MlDsaVerifier::verify(&msg, sig, &s1.public_key_bytes()));

    // Verify with s2's key fails
    assert!(!MlDsaVerifier::verify(&msg, sig, &s2.public_key_bytes()));
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 5: Signer from keystore
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn signer_from_keystore() {
    let mut store = KeyStore::new();
    let addr = store.generate_key("signer-test", "my-pass").unwrap();

    // Create signer via unlock
    let signer = WalletSigner::unlock(&store, "signer-test", "my-pass").unwrap();
    assert_eq!(*signer.address(), addr);

    // Sign and verify
    let msg = b"keystore-backed signing";
    let sig = signer.sign_bytes(msg);
    assert!(MlDsaVerifier::verify(msg, &sig, &signer.public_key_bytes()));

    // Sign a transaction
    let builder = TxBuilder::new(addr);
    let tx = builder.transfer([2u8; 32], 500, 0);
    let signed = signer.sign_for_chain(&tx, "evaporchain-testnet");
    assert!(signed.signature().is_some());
}

#[test]
fn signer_from_keystore_by_address() {
    let mut store = KeyStore::new();
    let addr = store.generate_key("by-addr", "pass").unwrap();

    let signer = WalletSigner::unlock_by_address(&store, &addr, "pass").unwrap();
    assert_eq!(*signer.address(), addr);

    let sig = signer.sign_bytes(b"test");
    assert!(MlDsaVerifier::verify(
        b"test",
        &sig,
        &signer.public_key_bytes()
    ));
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 6: Signature determinism — same key, same message → same sig
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn address_derivation_matches_pubkey() {
    let signer = make_signer();
    let expected = derive_address(&signer.public_key_bytes());
    assert_eq!(*signer.address(), expected);
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 7: sign_bytes — raw message signing
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn sign_bytes_various_payloads() {
    let signer = make_signer();
    let pk = signer.public_key_bytes();

    let payloads: Vec<&[u8]> = vec![
        b"",                     // empty
        b"a",                    // single byte
        b"hello evaporchain",    // short
        &[0u8; 1024],            // 1KB zeros
        &[0xFF; 4096],           // 4KB max bytes
        b"\x00\x01\x02\x03\x04", // binary
    ];

    for payload in payloads {
        let sig = signer.sign_bytes(payload);
        assert!(!sig.is_empty());
        assert!(
            MlDsaVerifier::verify(payload, &sig, &pk),
            "failed for payload len={}",
            payload.len()
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Scenario 8: Tampered message fails verification
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn tampered_message_fails_verification() {
    let signer = make_signer();
    let pk = signer.public_key_bytes();

    let original = b"send 1000 EVR to alice";
    let sig = signer.sign_bytes(original);

    // Original verifies
    assert!(MlDsaVerifier::verify(original, &sig, &pk));

    // Tampered message fails
    let tampered = b"send 9999 EVR to alice";
    assert!(!MlDsaVerifier::verify(tampered, &sig, &pk));

    // Even 1-bit change fails
    let mut one_bit_off = original.to_vec();
    one_bit_off[0] ^= 1;
    assert!(!MlDsaVerifier::verify(&one_bit_off, &sig, &pk));
}
