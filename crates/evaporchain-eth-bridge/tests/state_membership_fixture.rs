//! Generate the cross-side state-membership test fixture.
//!
//! Produces `ethereum-bridge/contracts/fixtures/state_membership_5.json`
//! with a finalised header at height H, plus an aggregate BLS attestation
//! over `(DOMAIN_TAG_STATE_MEMBERSHIP, H, key, keccak256(value))`.
//!
//! Consumed by `StateMembershipAttester.t.sol::test_realStateMembershipVerifies`.

use bls12_381::hash_to_curve::{ExpandMsgXmd, HashToCurve};
use bls12_381::{G1Affine, G1Projective, G2Affine, G2Projective, Scalar};
use evaporchain_eth_bridge::valset::{compute_root, Validator};
use group::Curve;
use serde_json::json;
use sha3::{Digest, Keccak256};
use std::fs;
use std::path::PathBuf;

const BLS_DST: &[u8] = b"BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_NUL_";
const DOMAIN_TAG_HEADER_TEXT: &str = "EvaporChain/v1/header";
const DOMAIN_TAG_STATE_MEMBERSHIP_TEXT: &str = "EvaporChain/v1/state-membership";

fn keygen_deterministic(seed: u64) -> (Scalar, G1Affine) {
    let mut bytes = [0u8; 64];
    bytes[0..8].copy_from_slice(&seed.to_le_bytes());
    for (i, byte) in bytes.iter_mut().enumerate().skip(8) {
        *byte = (seed.wrapping_mul(i as u64) & 0xFF) as u8;
    }
    let sk = Scalar::from_bytes_wide(&bytes);
    let pk: G1Affine = (G1Projective::generator() * sk).into();
    (sk, pk)
}

fn sign(sk: Scalar, msg: &[u8]) -> G2Affine {
    let h: G2Projective =
        <G2Projective as HashToCurve<ExpandMsgXmd<sha2_old_for_bls::Sha256>>>::hash_to_curve(
            msg, BLS_DST,
        );
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
    let raw = p.to_uncompressed();
    let mut out = [0u8; 128];
    out[16..64].copy_from_slice(&raw[0..48]);
    out[80..128].copy_from_slice(&raw[48..96]);
    out
}

fn g2_to_eip2537(p: G2Affine) -> [u8; 256] {
    let raw = p.to_uncompressed();
    let mut out = [0u8; 256];
    out[16..64].copy_from_slice(&raw[48..96]); // x.c0
    out[80..128].copy_from_slice(&raw[0..48]); // x.c1
    out[144..192].copy_from_slice(&raw[144..192]); // y.c0
    out[208..256].copy_from_slice(&raw[96..144]); // y.c1
    out
}

fn keccak(data: &[u8]) -> [u8; 32] {
    let mut h = Keccak256::new();
    h.update(data);
    let out = h.finalize();
    let mut a = [0u8; 32];
    a.copy_from_slice(&out);
    a
}

#[test]
fn generate_state_membership_fixture() {
    let epoch: u64 = 1;
    let height: u64 = 12_345;

    let stakes = [100u128; 5];
    let mut sks: Vec<Scalar> = Vec::new();
    let mut pks_compressed: Vec<[u8; 48]> = Vec::new();
    let mut pks_uncompressed: Vec<[u8; 128]> = Vec::new();
    for (i, _) in stakes.iter().enumerate() {
        let (sk, pk) = keygen_deterministic(0xC001D00DD0BAAA00 ^ (i as u64));
        sks.push(sk);
        pks_compressed.push(pk.to_compressed());
        pks_uncompressed.push(g1_to_eip2537(pk));
    }
    let validators: Vec<Validator> = pks_compressed
        .iter()
        .zip(stakes.iter())
        .map(|(pk, s)| Validator {
            pubkey: *pk,
            stake: *s,
        })
        .collect();
    let (_valset_root, _total) = compute_root(epoch, &validators).unwrap();

    // ── Header (so the inbox has stateRoot at this height) ──────
    let block_hash = keccak(format!("blk-{height}").as_bytes());
    let state_root = keccak(format!("state-{height}").as_bytes());
    let mmr_root = keccak(format!("mmr-{height}").as_bytes());

    let mut header_preimg = Vec::new();
    header_preimg.extend_from_slice(&keccak(DOMAIN_TAG_HEADER_TEXT.as_bytes()));
    header_preimg.extend_from_slice(&height.to_be_bytes());
    header_preimg.extend_from_slice(&block_hash);
    header_preimg.extend_from_slice(&state_root);
    header_preimg.extend_from_slice(&mmr_root);
    header_preimg.extend_from_slice(&epoch.to_be_bytes());
    let header_msg_hash = keccak(&header_preimg);

    let header_sigs: Vec<G2Affine> = sks.iter().map(|sk| sign(*sk, &header_msg_hash)).collect();
    let header_agg = g2_to_eip2537(aggregate(&header_sigs));

    // ── State-membership claim (signed separately) ───────────────
    //
    // Validators attest: at height=12345, `key` has `value`.
    let key = keccak(b"account_balance/0xCAFEBABE");
    let value = b"1000000000000000000".to_vec(); // 1e18 wei (string-encoded for the demo)
    let value_hash = keccak(&value);

    let mut attest_preimg = Vec::new();
    attest_preimg.extend_from_slice(&keccak(DOMAIN_TAG_STATE_MEMBERSHIP_TEXT.as_bytes()));
    attest_preimg.extend_from_slice(&height.to_be_bytes());
    attest_preimg.extend_from_slice(&key);
    attest_preimg.extend_from_slice(&value_hash);
    let attest_msg_hash = keccak(&attest_preimg);

    let attest_sigs: Vec<G2Affine> = sks.iter().map(|sk| sign(*sk, &attest_msg_hash)).collect();
    let attest_agg = g2_to_eip2537(aggregate(&attest_sigs));

    let bitmap = vec![0x1Fu8];
    let pks_uncomp_concat: Vec<u8> = pks_uncompressed
        .iter()
        .flat_map(|p| p.iter().copied())
        .collect();

    let fixture = json!({
        "epoch": epoch,
        "height": height,
        "block_hash": format!("0x{}", hex::encode(block_hash)),
        "state_root": format!("0x{}", hex::encode(state_root)),
        "mmr_root": format!("0x{}", hex::encode(mmr_root)),
        "validators": validators.iter().map(|v| json!({
            "pubkey_compressed": format!("0x{}", hex::encode(v.pubkey)),
            "stake": v.stake.to_string(),
        })).collect::<Vec<_>>(),
        "prev_pubkeys_uncompressed": format!("0x{}", hex::encode(&pks_uncomp_concat)),
        "signed_bitmap": format!("0x{}", hex::encode(&bitmap)),
        "header_agg_signature": format!("0x{}", hex::encode(header_agg)),

        // State-membership payload:
        "key": format!("0x{}", hex::encode(key)),
        "value": format!("0x{}", hex::encode(&value)),
        "value_hash": format!("0x{}", hex::encode(value_hash)),
        "attestation_agg_signature": format!("0x{}", hex::encode(attest_agg)),
    });

    let mut path: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop();
    path.pop();
    path.push("ethereum-bridge");
    path.push("contracts");
    path.push("fixtures");
    fs::create_dir_all(&path).unwrap();
    path.push("state_membership_5.json");

    fs::write(&path, serde_json::to_vec_pretty(&fixture).unwrap()).unwrap();
    eprintln!("wrote fixture: {}", path.display());
}
