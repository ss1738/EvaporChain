//! Generate the cross-side commit-cert test fixture.
//!
//! Produces `ethereum-bridge/contracts/fixtures/commit_cert_5.json`
//! containing 5 deterministic BLS keypairs, a hand-rolled valset
//! transition, an aggregate signature, and the message hash. The
//! Foundry test `CommitCertVerifier.t.sol::test_realBlsCommitCertVerifies`
//! reads this file via `vm.readFile`.
//!
//! This is the e2e Phase 2 cross-side check.

use bls12_381::hash_to_curve::{ExpandMsgXmd, HashToCurve};
use bls12_381::{G1Affine, G1Projective, G2Affine, G2Projective, Scalar};
use evaporchain_eth_bridge::valset::{compute_root, Validator};
use group::{Curve, Group};
use serde_json::json;
use sha3::{Digest, Keccak256};
use std::fs;
use std::path::PathBuf;

const BLS_DST: &[u8] = b"BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_NUL_";
const DOMAIN_TAG_COMMIT_TEXT: &str = "EvaporChain/v1/commit-cert";

fn keygen_deterministic(seed: u64) -> (Scalar, G1Affine) {
    let mut bytes = [0u8; 64];
    bytes[0..8].copy_from_slice(&seed.to_le_bytes());
    // Spread to fill bytes; keep deterministic.
    for i in 8..64 {
        bytes[i] = (seed.wrapping_mul(i as u64) & 0xFF) as u8;
    }
    let sk = Scalar::from_bytes_wide(&bytes);
    let pk: G1Affine = (G1Projective::generator() * sk).into();
    (sk, pk)
}

fn sign(sk: Scalar, msg: &[u8]) -> G2Affine {
    let h: G2Projective = <G2Projective as HashToCurve<
        ExpandMsgXmd<sha2_old_for_bls::Sha256>,
    >>::hash_to_curve(msg, BLS_DST);
    (h * sk).into()
}

fn aggregate(sigs: &[G2Affine]) -> G2Affine {
    let mut acc = G2Projective::identity();
    for s in sigs {
        acc += G2Projective::from(*s);
    }
    acc.to_affine()
}

fn g1_to_eip2537(p: G1Affine) -> [u8; 128] {
    let raw = p.to_uncompressed(); // [x(48) || y(48)]
    let mut out = [0u8; 128];
    out[16..64].copy_from_slice(&raw[0..48]);
    out[80..128].copy_from_slice(&raw[48..96]);
    out
}

fn g2_to_eip2537(p: G2Affine) -> [u8; 256] {
    // bls12_381 0.8: [x.c1, x.c0, y.c1, y.c0] × 48 bytes
    let raw = p.to_uncompressed();
    let x_c1 = &raw[0..48];
    let x_c0 = &raw[48..96];
    let y_c1 = &raw[96..144];
    let y_c0 = &raw[144..192];
    let mut out = [0u8; 256];
    out[16..64].copy_from_slice(x_c0);
    out[80..128].copy_from_slice(x_c1);
    out[144..192].copy_from_slice(y_c0);
    out[208..256].copy_from_slice(y_c1);
    out
}

fn keccak256(data: &[u8]) -> [u8; 32] {
    let mut h = Keccak256::new();
    h.update(data);
    let out = h.finalize();
    let mut a = [0u8; 32];
    a.copy_from_slice(&out);
    a
}

fn keccak_domain_tag(tag: &str) -> [u8; 32] {
    keccak256(tag.as_bytes())
}

#[test]
fn generate_commit_cert_5_fixture() {
    let prev_epoch: u64 = 1;
    let next_epoch: u64 = 2;

    // 5 validators with deterministic keys + per-validator stakes.
    let stakes = [100u128, 100, 100, 100, 100];
    let mut sks: Vec<Scalar> = Vec::new();
    let mut pks_compressed: Vec<[u8; 48]> = Vec::new();
    let mut pks_uncompressed: Vec<[u8; 128]> = Vec::new();

    for (i, _stake) in stakes.iter().enumerate() {
        let (sk, pk) = keygen_deterministic(0xC001D00DD0BAAA00 ^ (i as u64));
        sks.push(sk);
        pks_compressed.push(pk.to_compressed());
        pks_uncompressed.push(g1_to_eip2537(pk));
    }

    // Build prev valset & root.
    let prev_validators: Vec<Validator> = pks_compressed
        .iter()
        .zip(stakes.iter())
        .map(|(pk, s)| Validator {
            pubkey: *pk,
            stake: *s,
        })
        .collect();
    let (prev_root, prev_total) = compute_root(prev_epoch, &prev_validators).unwrap();
    assert_eq!(prev_total, 500);

    // Build next valset (rotate validator 4 — change stake to make root different).
    let mut next_validators = prev_validators.clone();
    next_validators[4].stake = 200;
    let (next_root, next_total) = compute_root(next_epoch, &next_validators).unwrap();
    assert_eq!(next_total, 600);

    // Message hash: keccak256(DOMAIN_TAG_COMMIT || prev_epoch || next_root || next_epoch).
    let dom = keccak_domain_tag(DOMAIN_TAG_COMMIT_TEXT);
    let mut preimg: Vec<u8> = Vec::new();
    preimg.extend_from_slice(&dom);
    preimg.extend_from_slice(&prev_epoch.to_be_bytes());
    preimg.extend_from_slice(&next_root);
    preimg.extend_from_slice(&next_epoch.to_be_bytes());
    let message_hash = keccak256(&preimg);

    // Each validator signs the 32-byte message_hash directly.
    let sigs: Vec<G2Affine> = sks.iter().map(|sk| sign(*sk, &message_hash)).collect();
    let agg = aggregate(&sigs);
    let agg_enc = g2_to_eip2537(agg);

    // Bitmap: all 5 signed → bits 0..4 set, LSB-first → 0x1F.
    let bitmap = vec![0x1Fu8];

    // Concatenate uncompressed pubkeys for the verifier.
    let prev_pks_uncomp_concat: Vec<u8> = pks_uncompressed
        .iter()
        .flat_map(|p| p.iter().copied())
        .collect();

    // Emit JSON fixture for Foundry.
    let fixture = json!({
        "prev_epoch": prev_epoch,
        "next_epoch": next_epoch,
        "prev_root": format!("0x{}", hex::encode(prev_root)),
        "next_root": format!("0x{}", hex::encode(next_root)),
        "message_hash": format!("0x{}", hex::encode(message_hash)),
        "validators": prev_validators.iter().map(|v| json!({
            "pubkey_compressed": format!("0x{}", hex::encode(v.pubkey)),
            "stake": v.stake.to_string(),
        })).collect::<Vec<_>>(),
        "next_validators": next_validators.iter().map(|v| json!({
            "pubkey_compressed": format!("0x{}", hex::encode(v.pubkey)),
            "stake": v.stake.to_string(),
        })).collect::<Vec<_>>(),
        "prev_pubkeys_uncompressed": format!("0x{}", hex::encode(&prev_pks_uncomp_concat)),
        "signed_bitmap": format!("0x{}", hex::encode(&bitmap)),
        "agg_signature_uncompressed": format!("0x{}", hex::encode(agg_enc)),
        "expected_total_stake_prev": prev_total.to_string(),
        "expected_total_stake_next": next_total.to_string(),
    });

    let mut path: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop(); // crates/
    path.pop(); // EvaporChain/
    path.push("ethereum-bridge");
    path.push("contracts");
    path.push("fixtures");
    fs::create_dir_all(&path).unwrap();
    path.push("commit_cert_5.json");

    fs::write(&path, serde_json::to_vec_pretty(&fixture).unwrap()).unwrap();
    eprintln!("wrote fixture: {}", path.display());
}
