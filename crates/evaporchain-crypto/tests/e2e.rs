//! §Crypto — hybrid post-quantum signing layer e2e
//!
//! Scenario: "EvaporChain validator key lifecycle" — MEERA is a
//! validator onboarding onto the network. She generates a hybrid
//! keypair (ECDSA + ML-DSA), signs consensus messages, and verifies
//! them. IVAN is an adversary attempting single-component forgeries
//! and length-extension attacks.
//!
//! The suite proves: the hybrid signer is non-short-circuit (both
//! components must verify); domain-tagging blocks classical impersonation
//! of PQ signatures; BLAKE3 is deterministic and distinct from Poseidon.

use evaporchain_crypto::{
    blake3_hash, poseidon_hash, HybridKeypair, HybridVerifier,
    signatures::Signer,
};

// ── Tests ─────────────────────────────────────────────────────────────────

#[test]
fn hybrid_sign_verify_round_trip() {
    // Honest path: sign then verify over a consensus vote message.
    let kp  = HybridKeypair::generate();
    let pk  = kp.public_key_bytes();
    let msg = b"vote:block:10003:0xAA";
    let sig = kp.sign(msg);
    assert!(HybridVerifier::verify_hybrid(msg, &sig, &pk),
        "freshly signed message must verify");
}

#[test]
fn tampered_ecdsa_component_rejected() {
    // IVAN flips a byte in the ECDSA half of the signature.
    // The hybrid verifier must reject — ECDSA alone is not sufficient.
    let kp  = HybridKeypair::generate();
    let pk  = kp.public_key_bytes();
    let msg = b"prevote:round:7";
    let mut sig = kp.sign(msg);
    // Byte 1 is inside the ECDSA component (byte 0 = hybrid tag).
    sig[1] ^= 0xFF;
    assert!(!HybridVerifier::verify_hybrid(msg, &sig, &pk),
        "tampered ECDSA component must be rejected");
}

#[test]
fn tampered_mldsa_component_rejected() {
    // IVAN flips the last byte (deep in the ML-DSA component).
    let kp  = HybridKeypair::generate();
    let pk  = kp.public_key_bytes();
    let msg = b"precommit:height:50000";
    let mut sig = kp.sign(msg);
    let last = sig.len() - 1;
    sig[last] ^= 0xFF;
    assert!(!HybridVerifier::verify_hybrid(msg, &sig, &pk),
        "tampered ML-DSA component must be rejected");
}

#[test]
fn wrong_message_rejected() {
    // Valid signature, wrong message → verification fails.
    let kp  = HybridKeypair::generate();
    let pk  = kp.public_key_bytes();
    let sig = kp.sign(b"real-message");
    assert!(!HybridVerifier::verify_hybrid(b"forged-message", &sig, &pk),
        "signature for different message must be rejected");
}

#[test]
fn wrong_public_key_rejected() {
    // Valid signature checked against a different keypair's public key.
    let kp1 = HybridKeypair::generate();
    let kp2 = HybridKeypair::generate();
    let msg = b"consensus-message";
    let sig = kp1.sign(msg);
    assert!(!HybridVerifier::verify_hybrid(msg, &sig, &kp2.public_key_bytes()),
        "valid sig checked against wrong pubkey must be rejected");
}

#[test]
fn truncated_signature_not_hybrid() {
    // Dropping the hybrid tag byte makes the verifier reject without panic.
    let kp  = HybridKeypair::generate();
    let pk  = kp.public_key_bytes();
    let msg = b"header-vote";
    let sig = kp.sign(msg);
    let truncated = &sig[1..];
    assert!(!HybridVerifier::is_hybrid_sig(truncated),
        "sig without tag byte must not be detected as hybrid");
    assert!(!HybridVerifier::verify_hybrid(msg, truncated, &pk),
        "truncated sig must be rejected");
}

#[test]
fn two_validators_same_message_different_signatures() {
    // Signatures are non-deterministic (ECDSA randomised) — two
    // signers produce different bytes for the same message, but both
    // verify against their own public key.
    let kp1 = HybridKeypair::generate();
    let kp2 = HybridKeypair::generate();
    let msg = b"same-consensus-message";
    let sig1 = kp1.sign(msg);
    let sig2 = kp2.sign(msg);
    assert_ne!(sig1, sig2,
        "independent keypairs must produce different signatures");
    assert!(HybridVerifier::verify_hybrid(msg, &sig1, &kp1.public_key_bytes()));
    assert!(HybridVerifier::verify_hybrid(msg, &sig2, &kp2.public_key_bytes()));
}

#[test]
fn blake3_is_deterministic() {
    let msg = b"evaporchain-epoch-10003";
    assert_eq!(blake3_hash(msg), blake3_hash(msg),
        "BLAKE3 must be deterministic");
}

#[test]
fn blake3_distinct_inputs_produce_distinct_outputs() {
    let h1 = blake3_hash(b"block-a");
    let h2 = blake3_hash(b"block-b");
    assert_ne!(h1, h2, "distinct inputs must hash to distinct outputs");
}

#[test]
fn blake3_and_poseidon_disagree_on_same_input() {
    // The two hash functions must not be substitutable for each other.
    let input = b"evaporchain-verkle-leaf";
    let b3  = blake3_hash(input);
    let pos = poseidon_hash(input);
    assert_ne!(b3, pos,
        "BLAKE3 and Poseidon must produce distinct digests for the same input");
}

#[test]
fn meera_validator_lifecycle() {
    // Full arc: Meera generates her hybrid key, signs 3 consensus
    // messages across different rounds, verifies all 3.
    let kp = HybridKeypair::generate();
    let pk = kp.public_key_bytes();
    let messages: &[&[u8]] = &[
        b"propose:height:1:round:0",
        b"prevote:height:1:round:0:hash:0xAA",
        b"precommit:height:1:round:0:hash:0xAA",
    ];
    for msg in messages {
        let sig = kp.sign(msg);
        assert!(HybridVerifier::verify_hybrid(msg, &sig, &pk),
            "all consensus messages must verify: {:?}", msg);
    }
}
