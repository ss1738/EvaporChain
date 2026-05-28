//! Emits a Groth16 fixture for the RecursionDeciderCircuit
//! (B-1/B-2 1C §7 step 1 — Section A LIVE), so the existing
//! `VerkleProofVerifier.sol` Foundry test can prove the on-chain
//! verification works on a REAL proof from the new circuit
//! shape, not just the synthetic dummy.
//!
//! Output: `ethereum-bridge/contracts/fixtures/recursion_decider_smoke.json`
//!
//! Same JSON layout as `smoke-fixture-emit`:
//!   proof            — "0x<512 hex>" — 256-byte EIP-197 proof
//!   public_inputs    — [] (Section A binds only via witness-commit
//!                          — sections_bcd_wired=false; sections B/C/D
//!                          will add (committed_hash_*, z0, zi) PIs)
//!   vk.alpha         — "0x<128 hex>"
//!   vk.beta/gamma/delta — "0x<256 hex>"
//!   vk.ic            — ["0x<128 hex>"] — IC[0] only (since 0 PIs)
//!
//! Uses n=4 real Grumpkin bases — small enough that setup+prove are
//! sub-second; the on-chain verify cost is independent of n (single
//! 4-pair EIP-197 pairing). The full n_aux=16,384 setup is
//! `recursion_decider_groth16_full_n_aux_16384` (the (d)-4 test).

use ark_bn254::{Fq as Bn254Fq, Fr as Bn254Fr, G1Affine, G2Affine};
use ark_ec::short_weierstrass::{Projective, SWCurveConfig};
use ark_ec::CurveGroup;
use ark_ff::{BigInteger, PrimeField};
use ark_std::rand::SeedableRng;

use evaporchain_nova_bridge::{
    eip197::proof_to_eip197_bytes,
    groth16_wrapper::{prove_recursion_decider, setup_recursion_decider},
    grumpkin_config::GrumpkinConfig,
    recursion_decider_circuit::RecursionDeciderCircuit,
};

fn fq_be32(f: &ark_bn254::Fq) -> [u8; 32] {
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

#[allow(deprecated)]
fn main() {
    let mut rng = ark_std::rand::rngs::StdRng::seed_from_u64(0);

    // n=4 real Grumpkin bases via doubling chain.
    let g = Projective::<GrumpkinConfig>::from(GrumpkinConfig::GENERATOR);
    let g2 = g + g;
    let g3 = g2 + g;
    let g5 = g3 + g2;
    let h = g + g + g + g + g + g + g; // 7G
    let bases: Vec<_> = [g, g2, g3, g5]
        .into_iter()
        .map(|p| p.into_affine())
        .collect();
    let h_aff = h.into_affine();

    eprintln!("recursion-decider-fixture-emit: setup_recursion_decider (n=4)…");
    let (pk, vk) = setup_recursion_decider(bases.clone(), h_aff, &mut rng).expect("setup");
    eprintln!(
        "recursion-decider-fixture-emit: ic.len()={} (1 + num_public_inputs = 1 + 0)",
        vk.gamma_abc_g1.len()
    );

    // Consistent witness.
    let scalars = vec![
        Bn254Fq::from(2u64),
        Bn254Fq::from(3u64),
        Bn254Fq::from(5u64),
        Bn254Fq::from(7u64),
    ];
    let blind = Bn254Fq::from(11u64);
    let claimed = g * scalars[0] + g2 * scalars[1] + g3 * scalars[2] + g5 * scalars[3] + h * blind;

    let circuit = RecursionDeciderCircuit::section_a_only(scalars, bases, blind, h_aff, claimed);

    eprintln!("recursion-decider-fixture-emit: prove …");
    let proof = prove_recursion_decider(&pk, circuit, &mut rng).expect("prove");
    let proof_bytes = proof_to_eip197_bytes(&proof);

    // Section A binds only via witness-commit ⇒ zero public inputs.
    let pi_arr: Vec<String> = vec![];

    let ic_arr: Vec<String> = vk.gamma_abc_g1.iter().map(|p| hex(&g1_bytes(p))).collect();

    let json = serde_json::json!({
        "proof": hex(&proof_bytes),
        "public_inputs": pi_arr,
        "vk": {
            "alpha": hex(&g1_bytes(&vk.alpha_g1)),
            "beta":  hex(&g2_bytes(&vk.beta_g2)),
            "gamma": hex(&g2_bytes(&vk.gamma_g2)),
            "delta": hex(&g2_bytes(&vk.delta_g2)),
            "ic":    ic_arr,
        }
    });

    let out_path = "ethereum-bridge/contracts/fixtures/recursion_decider_smoke.json";
    std::fs::write(out_path, serde_json::to_string_pretty(&json).unwrap()).expect("write fixture");
    eprintln!("recursion-decider-fixture-emit: wrote {out_path}");
    eprintln!(
        "recursion-decider-fixture-emit: proof_bytes={} public_inputs={} ic={}",
        proof_bytes.len(),
        pi_arr.len(),
        vk.gamma_abc_g1.len()
    );

    // Silence unused-import warning from Bn254Fr.
    let _ = Bn254Fr::from(1u64);
}
