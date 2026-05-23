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
/// # Rogue-key precondition
///
/// This function assumes that every key in `pk_byte_slices` has already
/// had its Proof-of-Possession (PoP) verified (e.g. at validator
/// registration via `BlsVerifier::verify_proof_of_possession`).
/// Callers that receive keys from untrusted sources — browser dApps,
/// light clients, indexers — MUST use [`aggregate_verify_with_pop`]
/// instead. Summing unverified G1 points is vulnerable to the rogue-key
/// attack (RFC 9380 §3.3): an adversary picks a "public key" that cancels
/// honest keys in the aggregated sum and produces forgeable signatures.
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

/// Verify a Proof-of-Possession for a G1 public key.
///
/// A PoP is a BLS signature of the key's own compressed bytes produced
/// under `BLS_POP_DST`. This proves the signer owns the corresponding
/// secret key, preventing rogue-key attacks at aggregation time.
///
/// Returns `true` iff the PoP is a valid G2 signature over `pk_bytes`
/// under `BLS_POP_DST` with `pk_bytes` as the signing key.
pub fn verify_pop(pk_bytes: &[u8], pop_bytes: &[u8]) -> bool {
    // H-4: PoP = sign(pk_bytes, BLS_POP_DST) under the same pk.
    verify(pk_bytes, pop_bytes, pk_bytes, crate::signatures::BLS_POP_DST)
}

/// Aggregate-verify with per-key Proof-of-Possession checks.
///
/// Safe drop-in for [`aggregate_verify`] when keys arrive from untrusted
/// sources. Verifies each key's PoP before summing in G1 — the check
/// closes the rogue-key window for browser dApps, light clients, and
/// indexers that cannot rely on validator-registration vetting.
///
/// `pk_pop_pairs` — slice of `(pk_bytes, pop_bytes)` tuples.
/// Returns `false` if any PoP is invalid or if the aggregate signature
/// does not verify.
pub fn aggregate_verify_with_pop(
    msg: &[u8],
    agg_sig_bytes: &[u8],
    pk_pop_pairs: &[(&[u8], &[u8])],
    dst: &[u8],
) -> bool {
    if pk_pop_pairs.is_empty() {
        return false;
    }
    // H-4: verify each key's PoP before aggregating.
    for (pk_bytes, pop_bytes) in pk_pop_pairs {
        if !verify_pop(pk_bytes, pop_bytes) {
            return false;
        }
    }
    let pk_slices: Vec<&[u8]> = pk_pop_pairs.iter().map(|(pk, _)| *pk).collect();
    aggregate_verify(msg, agg_sig_bytes, &pk_slices, dst)
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

    /// H-4: verify_pop must reject garbage/wrong-length PoP bytes.
    ///
    /// A real PoP is a G2 signature (96 bytes) over the public key's own
    /// compressed G1 bytes (48 bytes) under BLS_POP_DST. Supplying wrong
    /// lengths or all-zero blobs must return false.
    #[test]
    fn h4_verify_pop_rejects_invalid_pop_bytes() {
        let g1 = G1Affine::generator();
        let pk_bytes = g1.to_compressed().to_vec(); // 48-byte valid G1 pk

        // Wrong length PoP.
        assert!(!verify_pop(&pk_bytes, &[0u8; 95]));
        // Correct length but all-zero G2 (not a valid curve point).
        assert!(!verify_pop(&pk_bytes, &[0u8; 96]));
        // Correct length, non-zero garbage.
        assert!(!verify_pop(&pk_bytes, &[0xABu8; 96]));
    }

    /// H-4: aggregate_verify_with_pop must reject keys that lack a valid PoP.
    ///
    /// A caller supplying unverified keys to aggregate_verify is exposed to
    /// the rogue-key attack. aggregate_verify_with_pop closes this by
    /// rejecting any key whose PoP does not verify before summing in G1.
    #[test]
    fn h4_aggregate_verify_with_pop_rejects_missing_pop() {
        let g1 = G1Affine::generator();
        let pk_bytes = g1.to_compressed().to_vec();
        let bad_pop = vec![0xFFu8; 96]; // not a valid G2 point signature
        let agg_sig = vec![0u8; 96];

        let result = aggregate_verify_with_pop(
            b"msg",
            &agg_sig,
            &[(&pk_bytes, &bad_pop)],
            b"dst",
        );
        assert!(!result, "must reject aggregate when any PoP is invalid");
    }
}
