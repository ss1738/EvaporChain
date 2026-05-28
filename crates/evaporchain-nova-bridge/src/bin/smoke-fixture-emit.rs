//! Emits a deterministic Groth16 smoke-test fixture for
//! `VerkleProofVerifier.t.sol`.
//!
//! Uses `NovaVerifierCircuit::dummy()` (zero witnesses, satisfied
//! by vacuous constraints) + `setup(seed=0)` + `prove(seed=0)`.
//!
//! Output: `ethereum-bridge/contracts/fixtures/verkle_proof_smoke.json`
//!
//! JSON layout:
//!   proof            — "0x<512 hex>" — 256-byte EIP-197 proof
//!   public_inputs    — ["0x<64 hex>", ...]   — 4 uint256 values (BE)
//!   vk.alpha         — "0x<128 hex>"  — G1 (x||y, 64 bytes BE)
//!   vk.beta          — "0x<256 hex>"  — G2 (xc1||xc0||yc1||yc0, 128 bytes BE)
//!   vk.gamma         — same as beta
//!   vk.delta         — same as beta
//!   vk.ic            — ["0x<128 hex>", ...] — IC[0..n] G1 points
//!
//! Solidity decodes each value via:
//!   `abi.decode(vm.parseJsonBytes(json, ".field"), (uint256, uint256))`
//! since the raw 32-byte-BE limbs are exactly the ABI encoding of
//! uint256 pairs/quads.

use ark_bn254::{Fq, G1Affine, G2Affine};
use ark_ff::{BigInteger, PrimeField};
use ark_std::rand::SeedableRng;

use evaporchain_nova_bridge::{
    eip197::proof_to_eip197_bytes,
    groth16_wrapper::{prove, public_inputs_for, setup},
    verifier_circuit::NovaVerifierCircuit,
};

fn fq_be32(f: &Fq) -> [u8; 32] {
    let le = f.into_bigint().to_bytes_le();
    let mut out = [0u8; 32];
    for (i, b) in le.iter().take(32).enumerate() {
        out[31 - i] = *b;
    }
    out
}

fn g1_bytes(p: &G1Affine) -> Vec<u8> {
    let mut v = Vec::with_capacity(64);
    v.extend_from_slice(&fq_be32(&p.x));
    v.extend_from_slice(&fq_be32(&p.y));
    v
}

fn g2_bytes(p: &G2Affine) -> Vec<u8> {
    // EIP-197 / pairing-precompile order: xc1, xc0, yc1, yc0
    let mut v = Vec::with_capacity(128);
    v.extend_from_slice(&fq_be32(&p.x.c1));
    v.extend_from_slice(&fq_be32(&p.x.c0));
    v.extend_from_slice(&fq_be32(&p.y.c1));
    v.extend_from_slice(&fq_be32(&p.y.c0));
    v
}

fn hex(b: &[u8]) -> String {
    format!("0x{}", hex::encode(b))
}

fn main() {
    let mut rng = ark_std::rand::rngs::StdRng::seed_from_u64(0);

    eprintln!("smoke-fixture-emit: setup(seed=0) …");
    let (pk, vk) = setup(&mut rng).expect("setup");
    eprintln!(
        "smoke-fixture-emit: ic.len()={} (= 1 + num_public_inputs)",
        vk.gamma_abc_g1.len()
    );

    let dummy = NovaVerifierCircuit::dummy();
    let public_inputs = public_inputs_for(&dummy);
    eprintln!(
        "smoke-fixture-emit: public_inputs.len()={}",
        public_inputs.len()
    );

    eprintln!("smoke-fixture-emit: prove(seed=0) …");
    let proof = prove(&pk, dummy, &mut rng).expect("prove");
    let proof_bytes = proof_to_eip197_bytes(&proof);

    // Build JSON manually to control order
    let pi_arr: Vec<String> = public_inputs
        .iter()
        .map(|f| {
            let le = f.into_bigint().to_bytes_le();
            let mut be = [0u8; 32];
            for (i, b) in le.iter().take(32).enumerate() {
                be[31 - i] = *b;
            }
            hex(&be)
        })
        .collect();

    let ic_arr: Vec<String> = vk.gamma_abc_g1.iter().map(|p| hex(&g1_bytes(p))).collect();

    let json = serde_json::json!({
        "proof": hex(&proof_bytes),
        "public_inputs": pi_arr,
        "vk": {
            "alpha": hex(&g1_bytes(&vk.alpha_g1)),
            "beta":  hex(&g2_bytes(&vk.beta_g2)),
            "gamma": hex(&g2_bytes(&vk.gamma_g2)),
            "delta": hex(&g2_bytes(&vk.delta_g2)),
            "ic":    ic_arr
        }
    });

    let out_path = "ethereum-bridge/contracts/fixtures/verkle_proof_smoke.json";
    std::fs::write(out_path, serde_json::to_string_pretty(&json).unwrap()).expect("write fixture");
    eprintln!("smoke-fixture-emit: wrote {out_path}");
    eprintln!(
        "smoke-fixture-emit: proof_bytes={} public_inputs={} ic={}",
        proof_bytes.len(),
        public_inputs.len(),
        vk.gamma_abc_g1.len()
    );
}
