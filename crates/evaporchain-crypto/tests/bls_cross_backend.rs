//! Cross-backend interop tests: blst (bls-native) vs bls12_381 (bls-portable).
//!
//! Refactor B (commit `99bab9c`) added a pure-Rust BLS verifier so the
//! Light Client SDK's WASM bridge can run BFT BLS aggregate-sig
//! verification in browsers without pulling blst (a C library that
//! doesn't compile to wasm32). The portable verifier was implemented
//! against the RFC-9380 hash-to-curve + RFC-9381 pairing-equation
//! conventions blst uses — but until this test, it was never confirmed
//! to produce bit-identical results on real signatures.
//!
//! These tests REQUIRE both features enabled simultaneously:
//!
//!     cargo test -p evaporchain-crypto --features bls-portable --tests
//!
//! (bls-native is the default feature, so adding bls-portable enables both.)
//!
//! Each test uses `BlsKeypair` (gated to `bls-native`) to produce real
//! signatures via blst, then calls the portable `bls_portable::verify`
//! / `aggregate_verify` directly to confirm cross-backend agreement.
//!
//! If any test fails: the portable verifier disagrees with blst on real
//! data, browsers running the WASM bridge would silently accept invalid
//! signatures or reject valid ones. Mainnet-blocker.

#![cfg(all(feature = "bls-native", feature = "bls-portable"))]

use evaporchain_crypto::bls_portable;
use evaporchain_crypto::signatures::{BlsKeypair, BlsVerifier};

const BLS_DST: &[u8] = b"BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_NUL_";
const BLS_POP_DST: &[u8] = b"BLS_POP_BLS12381G2_XMD:SHA-256_SSWU_RO_POP_";
const BLS_ROTATION_DST: &[u8] = b"BLS_ROTATION_BLS12381G2_XMD:SHA-256_SSWU_RO_ROT_";

#[test]
fn portable_verifies_native_signed_message() {
    // Sign with blst.
    let kp = BlsKeypair::generate();
    let msg = b"hello evaporchain - cross-backend interop";
    let sig = kp.sign(msg);
    let pk = kp.public_key_bytes();

    // Sanity: native verifies its own sig.
    assert!(BlsVerifier::verify(msg, &sig, &pk));

    // Critical: portable verifies the same blst-signed sig.
    assert!(
        bls_portable::verify(msg, &sig.0, &pk.0, BLS_DST),
        "portable backend FAILED to verify a real blst-signed signature \
         — RFC-9380 hash-to-curve or pairing-equation mismatch with blst. \
         WASM verifier would silently break in browsers."
    );
}

#[test]
fn portable_rejects_tampered_signature() {
    let kp = BlsKeypair::generate();
    let msg = b"original message";
    let mut sig = kp.sign(msg);
    let pk = kp.public_key_bytes();

    // Flip a bit in the signature — must fail in both backends.
    sig.0[5] ^= 0x01;
    assert!(!BlsVerifier::verify(msg, &sig, &pk));
    assert!(!bls_portable::verify(msg, &sig.0, &pk.0, BLS_DST));
}

#[test]
fn portable_rejects_wrong_message() {
    let kp = BlsKeypair::generate();
    let original_msg = b"signed message";
    let wrong_msg = b"DIFFERENT message";
    let sig = kp.sign(original_msg);
    let pk = kp.public_key_bytes();

    // Native rejects.
    assert!(!BlsVerifier::verify(wrong_msg, &sig, &pk));
    // Portable must agree.
    assert!(!bls_portable::verify(wrong_msg, &sig.0, &pk.0, BLS_DST));
}

#[test]
fn portable_rejects_wrong_public_key() {
    let kp1 = BlsKeypair::generate();
    let kp2 = BlsKeypair::generate();
    let msg = b"signed by kp1";
    let sig = kp1.sign(msg);
    let wrong_pk = kp2.public_key_bytes();

    assert!(!BlsVerifier::verify(msg, &sig, &wrong_pk));
    assert!(!bls_portable::verify(msg, &sig.0, &wrong_pk.0, BLS_DST));
}

#[test]
fn portable_verifies_proof_of_possession() {
    // PoP uses BLS_POP_DST instead of BLS_DST. Test that the portable
    // verifier honors the DST swap correctly.
    let kp = BlsKeypair::generate();
    let pop = kp.proof_of_possession();
    let pk = kp.public_key_bytes();

    // Native verifies its own PoP.
    assert!(BlsVerifier::verify_proof_of_possession(&pk, &pop));

    // Portable must agree — message bytes are pk.0, DST is BLS_POP_DST.
    assert!(
        bls_portable::verify(&pk.0, &pop.0, &pk.0, BLS_POP_DST),
        "portable verifier rejected a real blst-produced PoP — \
         BLS_POP_DST handling diverges between backends"
    );
}

#[test]
fn portable_pop_rejects_wrong_dst() {
    // Cross-domain replay defense: a regular signature should NOT verify
    // as a PoP, because the DST differs. Both backends must enforce this.
    let kp = BlsKeypair::generate();
    let msg = b"regular vote";
    let regular_sig = kp.sign(msg); // Signed with BLS_DST
    let pk = kp.public_key_bytes();

    // Native rejects: trying to verify a regular sig as a PoP fails.
    assert!(!BlsVerifier::verify_proof_of_possession(&pk, &regular_sig));
    // Portable: same reasoning, called directly with BLS_POP_DST over pk bytes.
    assert!(!bls_portable::verify(&pk.0, &regular_sig.0, &pk.0, BLS_POP_DST));
}

#[test]
fn portable_verifies_rotation_continuity_proof() {
    let kp = BlsKeypair::generate();
    let new_pk_bytes = b"the new public key bytes - would be a real 48-byte G1 in production";
    let rotation_sig = kp.sign_rotation_continuity(new_pk_bytes);
    let pk = kp.public_key_bytes();

    // Native verifies.
    assert!(BlsVerifier::verify_rotation_continuity(
        &pk,
        new_pk_bytes,
        &rotation_sig
    ));
    // Portable must agree under BLS_ROTATION_DST.
    assert!(
        bls_portable::verify(new_pk_bytes, &rotation_sig.0, &pk.0, BLS_ROTATION_DST),
        "portable verifier rejected a real blst-produced rotation continuity proof"
    );
}

#[test]
fn portable_aggregate_verify_three_signers_native() {
    // Aggregate verify is the hot path for BFT consensus — many validators
    // each sign the same precommit message, signatures combine into one
    // aggregate, the verifier checks the aggregate against the public-key
    // sum. Browsers run this every block via the WASM bridge.
    let msg = b"precommit bytes for block N";
    let kps: Vec<BlsKeypair> = (0..3).map(|_| BlsKeypair::generate()).collect();
    let sigs = kps.iter().map(|kp| kp.sign(msg)).collect::<Vec<_>>();
    let pks = kps.iter().map(|kp| kp.public_key_bytes()).collect::<Vec<_>>();

    let agg_sig = BlsVerifier::aggregate_signatures(&sigs)
        .expect("native aggregation must succeed");

    // Native verifies its own aggregate.
    assert!(BlsVerifier::aggregate_verify(msg, &agg_sig, &pks));

    // Portable must agree on the same aggregate signature + same pks.
    let pk_byte_slices: Vec<&[u8]> = pks.iter().map(|p| p.0.as_slice()).collect();
    assert!(
        bls_portable::aggregate_verify(msg, &agg_sig.0, &pk_byte_slices, BLS_DST),
        "portable aggregate_verify FAILED to verify a 3-signer blst aggregate. \
         BFT consensus verification would break in browsers."
    );
}

#[test]
fn portable_aggregate_verify_rejects_missing_signer() {
    // If a signer's pk is omitted from the aggregate-verify call, both
    // backends must fail.
    let msg = b"4-signer aggregate, omit 1 pk";
    let kps: Vec<BlsKeypair> = (0..4).map(|_| BlsKeypair::generate()).collect();
    let sigs = kps.iter().map(|kp| kp.sign(msg)).collect::<Vec<_>>();
    let pks = kps.iter().map(|kp| kp.public_key_bytes()).collect::<Vec<_>>();

    let agg_sig = BlsVerifier::aggregate_signatures(&sigs).unwrap();

    // Drop the last pk.
    let truncated_pks: Vec<_> = pks[..3].to_vec();
    assert!(!BlsVerifier::aggregate_verify(msg, &agg_sig, &truncated_pks));

    let truncated_byte_slices: Vec<&[u8]> = truncated_pks.iter().map(|p| p.0.as_slice()).collect();
    assert!(!bls_portable::aggregate_verify(
        msg,
        &agg_sig.0,
        &truncated_byte_slices,
        BLS_DST
    ));
}

#[test]
fn portable_aggregate_verify_rejects_wrong_message() {
    let original_msg = b"signed bytes";
    let wrong_msg = b"different bytes";
    let kps: Vec<BlsKeypair> = (0..2).map(|_| BlsKeypair::generate()).collect();
    let sigs = kps.iter().map(|kp| kp.sign(original_msg)).collect::<Vec<_>>();
    let pks = kps.iter().map(|kp| kp.public_key_bytes()).collect::<Vec<_>>();
    let agg_sig = BlsVerifier::aggregate_signatures(&sigs).unwrap();

    assert!(!BlsVerifier::aggregate_verify(wrong_msg, &agg_sig, &pks));
    let pk_byte_slices: Vec<&[u8]> = pks.iter().map(|p| p.0.as_slice()).collect();
    assert!(!bls_portable::aggregate_verify(
        wrong_msg,
        &agg_sig.0,
        &pk_byte_slices,
        BLS_DST
    ));
}
