//! Portable (pure-Rust) BLS12-381 verifier backend.
//!
//! Activated by the `bls-portable` feature. Implements verify +
//! aggregate_verify against the same RFC-9380 hash-to-curve + RFC-9381
//! pairing-equation conventions blst uses, so signatures verified by
//! either backend produce identical results.
//!
//! Wire format: G1 public key compressed (48 bytes), G2 signature
//! compressed (96 bytes). Same as blst's `min_pk` mode — chain-side
//! signatures produced by `bls-native` validators verify here in
//! browser-side `bls-portable` consumers without any wire-format
//! conversion.
//!
//! Verification only — signing requires the `bls-native` feature
//! because browsers / dapps don't sign BLS.

#![cfg(feature = "bls-portable")]

use bls12_381::{
    hash_to_curve::{ExpandMsgXmd, HashToCurve},
    G1Affine, G1Projective, G2Affine, G2Projective,
};
use group::{prime::PrimeCurveAffine, Curve, Group};

/// Hash a message to G2 with the supplied DST. Implements the same
/// hash-to-curve scheme blst uses (RFC 9380, suite
/// `BLS12381G2_XMD:SHA-256_SSWU_RO_`).
///
/// Uses sha2 0.9 (renamed `sha2_old_for_bls`) because `bls12_381 0.8`'s
/// `ExpandMsgXmd<H>` requires the older digest 0.9 trait set; the
/// workspace's main `sha2 = "0.10"` uses incompatible digest 0.10
/// traits. See Cargo.toml note.
fn hash_to_g2(msg: &[u8], dst: &[u8]) -> G2Projective {
    <G2Projective as HashToCurve<ExpandMsgXmd<sha2_old_for_bls::Sha256>>>::hash_to_curve(msg, dst)
}

/// Parse a 48-byte compressed G1 point. Returns `None` on invalid
/// encoding or non-prime-order subgroup membership.
fn parse_g1_pk(bytes: &[u8]) -> Option<G1Affine> {
    if bytes.len() != 48 {
        return None;
    }
    let mut buf = [0u8; 48];
    buf.copy_from_slice(bytes);
    G1Affine::from_compressed(&buf).into_option()
}

/// Parse a 96-byte compressed G2 point.
fn parse_g2_sig(bytes: &[u8]) -> Option<G2Affine> {
    if bytes.len() != 96 {
        return None;
    }
    let mut buf = [0u8; 96];
    buf.copy_from_slice(bytes);
    G2Affine::from_compressed(&buf).into_option()
}

/// Verify a single BLS signature.
///
/// Verification equation: e(G1::generator(), sig) == e(pk, hash_to_g2(msg, dst)).
/// Equivalent to blst's `Signature::verify`.
pub fn verify(msg: &[u8], sig_bytes: &[u8], pk_bytes: &[u8], dst: &[u8]) -> bool {
    let pk = match parse_g1_pk(pk_bytes) {
        Some(p) => p,
        None => return false,
    };
    let sig = match parse_g2_sig(sig_bytes) {
        Some(s) => s,
        None => return false,
    };
    let h = hash_to_g2(msg, dst);
    let h_aff = h.to_affine();

    // Pairing-equation form: e(G1, sig) ?= e(pk, h)
    // bls12_381's pairing(g1, g2) returns Gt; equality checked directly.
    let g1_gen = G1Affine::generator();
    let lhs = bls12_381::pairing(&g1_gen, &sig);
    let rhs = bls12_381::pairing(&pk, &h_aff);
    lhs == rhs
}

/// Verify an aggregate signature: one G2 signature, multiple G1 public
/// keys, all signing the SAME message. Equivalent to blst's
/// `fast_aggregate_verify`.
///
/// fast_aggregate_verify works by aggregating the public keys (sum in
/// G1) and then doing a single pairing check:
///   e(G1::generator(), agg_sig) == e(sum(pks), hash_to_g2(msg, dst))
///
/// **H-4 (audit 2026-05-17) — caller responsibility for proof-of-possession.**
///
/// This function does NOT verify proofs-of-possession for the supplied
/// public keys. The caller MUST have independently verified PoP for
/// every `pk` in `pk_byte_slices`; otherwise the function is vulnerable
/// to the standard BLS rogue-key attack (Boneh-Drijvers-Neven 2018 §3):
/// an adversary registers `pk_adv = -sum(pk_honest_i) + pk_real_adv`,
/// then forges an aggregate signature on any message by signing only
/// with `pk_real_adv` — the aggregate sums to `pk_real_adv` so the
/// verification passes despite no honest signer having actually signed.
///
/// EvaporChain's validator path gates PoP at registration (W7 closure
/// in `BlsVerifier::verify_proof_of_possession`), so consensus-internal
/// calls are safe. Browser dApps, light clients, indexers, or any other
/// non-validator caller MUST call `BlsVerifier::verify_proof_of_possession`
/// for every key before this function. Until they do, treat the API as
/// `aggregate_verify_assuming_pop`.
pub fn aggregate_verify(
    msg: &[u8],
    agg_sig_bytes: &[u8],
    pk_byte_slices: &[&[u8]],
    dst: &[u8],
) -> bool {
    if pk_byte_slices.is_empty() {
        return false;
    }
    // Parse all pks; reject if any is invalid.
    let mut agg_pk = G1Projective::identity();
    for pk_bytes in pk_byte_slices {
        match parse_g1_pk(pk_bytes) {
            Some(p) => agg_pk += G1Projective::from(p),
            None => return false,
        }
    }
    let agg_pk_aff = agg_pk.to_affine();

    let sig = match parse_g2_sig(agg_sig_bytes) {
        Some(s) => s,
        None => return false,
    };
    let h = hash_to_g2(msg, dst).to_affine();

    let g1_gen = G1Affine::generator();
    let lhs = bls12_381::pairing(&g1_gen, &sig);
    let rhs = bls12_381::pairing(&agg_pk_aff, &h);
    lhs == rhs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_pk_bytes_rejected() {
        let pk_bad = vec![0u8; 47]; // wrong length
        assert!(!verify(b"msg", &[0u8; 96], &pk_bad, b"dst"));
    }

    #[test]
    fn invalid_sig_bytes_rejected() {
        // Use the G1 generator as a valid pk
        let g1 = G1Affine::generator();
        let pk_bytes = g1.to_compressed().to_vec();
        let sig_bad = vec![0u8; 95]; // wrong length
        assert!(!verify(b"msg", &sig_bad, &pk_bytes, b"dst"));
    }

    #[test]
    fn empty_aggregate_verify_returns_false() {
        let agg_sig = vec![0u8; 96];
        assert!(!aggregate_verify(b"msg", &agg_sig, &[], b"dst"));
    }
}
