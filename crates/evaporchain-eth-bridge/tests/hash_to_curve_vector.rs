//! Cross-side hash-to-G2 agreement.
//!
//! Computes `hash_to_g2(msg, BLS_DST)` using the exact same code path
//! as `evaporchain-crypto::bls_portable` (bls12_381 0.8 + ExpandMsgXmd<Sha256>),
//! emits the resulting G2 point in EIP-2537 uncompressed encoding (256 bytes),
//! and compares against the byte string produced by the Solidity-side
//! `HashToCurve.hashToG2` test (`test_dumpHashToG2_helloEvaporchain`).
//!
//! Format note (EIP-2537):
//!   G2 = (x, y) where x, y ∈ FP2.
//!   FP2 = (c0, c1) where c0, c1 ∈ FP. Order in encoding: c0 first, then c1.
//!   Each FP is 64 bytes: 16 zero bytes prefix || 48-byte big-endian value.
//!   Total: 4 × 64 = 256 bytes laid out as [x.c0, x.c1, y.c0, y.c1].
//!
//! `bls12_381 0.8` G2Affine.to_uncompressed() produces 192 bytes
//!   [x.c1 (48) || x.c0 (48) || y.c1 (48) || y.c0 (48)] — note c1 first!
//! We must transpose to EIP-2537's c0-first layout AND pad each 48 → 64.

use bls12_381::hash_to_curve::{ExpandMsgXmd, HashToCurve};
use bls12_381::{G2Affine, G2Projective};

const BLS_DST: &[u8] = b"BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_NUL_";

/// EIP-2537 uncompressed encoding of a `G2Affine`.
/// Returns 256 bytes laid out as [x.c0_64, x.c1_64, y.c0_64, y.c1_64].
fn g2_to_eip2537(p: G2Affine) -> [u8; 256] {
    // bls12_381 0.8 uncompressed layout: [x.c1, x.c0, y.c1, y.c0], 48 bytes each.
    let raw = p.to_uncompressed(); // [u8; 192]
    let x_c1 = &raw[0..48];
    let x_c0 = &raw[48..96];
    let y_c1 = &raw[96..144];
    let y_c0 = &raw[144..192];

    let mut out = [0u8; 256];
    // EIP-2537: x.c0, x.c1, y.c0, y.c1 — each as 16-byte zero pad + 48-byte BE.
    out[16..64].copy_from_slice(x_c0);
    out[80..128].copy_from_slice(x_c1);
    out[144..192].copy_from_slice(y_c0);
    out[208..256].copy_from_slice(y_c1);
    out
}

fn rust_hash_to_g2(msg: &[u8]) -> [u8; 256] {
    let p: G2Projective =
        <G2Projective as HashToCurve<ExpandMsgXmd<sha2_old_for_bls::Sha256>>>::hash_to_curve(
            msg, BLS_DST,
        );
    let aff: G2Affine = p.into();
    g2_to_eip2537(aff)
}

/// The Solidity-side test prints this exact value for the message
/// "hello evaporchain". If the two ever diverge, both sides need
/// audit (DST mismatch, encoding bug, or sha2 variant mismatch).
const SOL_HELLO_EVAPORCHAIN: &str = concat!(
    "00000000000000000000000000000000025b4ce58e3055c087643949e4537293",
    "216eea8d55c72fdb39e7758bd1e19f36e3b175b8a3e68abcbe4449d43d782068",
    "000000000000000000000000000000000aea8443dc2b25dd02e142df19d232b6",
    "b55fe39717d0c3d7f819cf077d0d056c0fb46b43ba9433fa2624a6a1f79017e6",
    "00000000000000000000000000000000137cde909f4c3c5f262a03dec7f58978",
    "f7e3591f23e275ce4f09d770c4b883f75180dd427482f8c0f0555c9a1577000c",
    "0000000000000000000000000000000006115c851ac0dacb9b33932153edc737",
    "f6dc5efcf194776db345086166adf336c52c5d5bac63bac7e26c9aae9cd12a2b",
);

#[test]
fn rust_matches_solidity_hello_evaporchain() {
    let rust = rust_hash_to_g2(b"hello evaporchain");
    let sol = hex::decode(SOL_HELLO_EVAPORCHAIN).unwrap();
    assert_eq!(sol.len(), 256);
    assert_eq!(
        rust.to_vec(),
        sol,
        "Rust hash_to_g2 disagrees with Solidity"
    );
}
