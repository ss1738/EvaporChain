//! Emits a Groth16 fixture for `RecursionDeciderCircuit` with the
//! Section B interface populated (the delegation-aware path per
//! dossier §6b). Parallels `recursion-decider-fixture-emit` but
//! threads through:
//!
//!   1. Real (pp, rs) via shared-pp helper.
//!   2. `assemble_section_b_pi_bundle` (verify gate).
//!   3. `bundle.into_section_b_pis()` → in-circuit Section B PI bundle.
//!   4. `setup_recursion_decider_with_b_interface(bases, h, pi_arity)`.
//!   5. `section_a_with_b_interface` circuit + prove.
//!   6. Emit JSON fixture with 11-element public_inputs.
//!
//! Output: `ethereum-bridge/contracts/fixtures/recursion_decider_b_smoke.json`
//!
//! Foundry test `RecursionDeciderBVerifierTest` loads this and
//! deploys `VerkleProofVerifier` with IC_LEN=12 (= 1 + 11 PIs).

use ark_bn254::{Fq as Bn254Fq, Fr as Bn254Fr, G1Affine, G2Affine};
use ark_ec::short_weierstrass::{Projective, SWCurveConfig};
use ark_ec::CurveGroup;
use ark_ff::{BigInteger, PrimeField};
use ark_std::rand::SeedableRng;
use ff::Field;

use evaporchain_nova_bridge::{
    eip197::proof_to_eip197_bytes,
    groth16_wrapper::{
        prove_recursion_decider, section_b_public_inputs_slice,
        setup_recursion_decider_with_b_interface,
    },
    grumpkin_config::GrumpkinConfig,
    l_u_secondary_extract::assemble_section_b_pi_bundle,
    recursion_decider_circuit::RecursionDeciderCircuit,
    recursive_snark_fixture::{Scalar1, TrivialIncrementCircuit, E1, E2},
};
use nova_snark::nova::{PublicParams, RecursiveSNARK};
use nova_snark::provider::hyperkzg::EvaluationEngine;
use nova_snark::provider::ipa_pc::EvaluationEngine as IpaEE;
use nova_snark::spartan::ppsnark::RelaxedR1CSSNARK;
use nova_snark::traits::snark::RelaxedR1CSSNARKTrait;

fn fq_be32(f: &Bn254Fq) -> [u8; 32] {
    let le = f.into_bigint().to_bytes_le();
    let mut out = [0u8; 32];
    for (i, b) in le.iter().take(32).enumerate() {
        out[31 - i] = *b;
    }
    out
}

fn fr_be32(f: &Bn254Fr) -> [u8; 32] {
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
    let mut rng = ark_std::rand::rngs::StdRng::seed_from_u64(0xBB);

    // ── 1. Build (pp, rs) with shared pp ────────────────────────────
    let circuit = TrivialIncrementCircuit;
    type S1 = RelaxedR1CSSNARK<E1, EvaluationEngine<E1>>;
    type S2 = RelaxedR1CSSNARK<E2, IpaEE<E2>>;
    let pp = PublicParams::<E1, E2, TrivialIncrementCircuit>::setup(
        &circuit,
        &*S1::ck_floor(),
        &*S2::ck_floor(),
    )
    .expect("pp setup");
    let z0_nova: Vec<Scalar1> = vec![Scalar1::ZERO];
    let mut rs = RecursiveSNARK::<E1, E2, TrivialIncrementCircuit>::new(&pp, &circuit, &z0_nova)
        .expect("rs new");
    for _ in 0..2 {
        rs.prove_step(&pp, &circuit).expect("prove_step");
    }
    eprintln!("recursion-decider-b-fixture-emit: 2-step fixture built");

    // ── 2. Off-chain adapter ────────────────────────────────────────
    let z0_ark = vec![Bn254Fr::from(0u64)];
    let bundle =
        assemble_section_b_pi_bundle(&pp, &rs, 2, &z0_ark).expect("assemble bundle (verify gate)");
    eprintln!("recursion-decider-b-fixture-emit: bundle verified; pi_count=9+|z0|+|zn|=11");

    // ── 3. Convert to in-circuit PIs ────────────────────────────────
    let section_b_pis = bundle.into_section_b_pis();
    let pi_arity = section_b_pis.z0.len();
    assert_eq!(pi_arity, 1, "TrivialIncrementCircuit pi_arity = 1");

    // ── 4. Setup Groth16 at Section A + B PI shape ──────────────────
    let g = Projective::<GrumpkinConfig>::from(GrumpkinConfig::GENERATOR);
    let g2_pt = g + g;
    let g3 = g2_pt + g;
    let g5 = g3 + g2_pt;
    let h = g + g + g + g + g + g + g;
    let bases: Vec<_> = [g, g2_pt, g3, g5]
        .into_iter()
        .map(|p| p.into_affine())
        .collect();
    let h_aff = h.into_affine();

    eprintln!("recursion-decider-b-fixture-emit: Groth16 setup …");
    let (pk, vk) =
        setup_recursion_decider_with_b_interface(bases.clone(), h_aff, pi_arity, &mut rng)
            .expect("setup");
    eprintln!(
        "recursion-decider-b-fixture-emit: ic.len()={} (1 + {} PIs)",
        vk.gamma_abc_g1.len(),
        vk.gamma_abc_g1.len() - 1,
    );

    // ── 5. Build circuit + prove ────────────────────────────────────
    let scalars = vec![
        Bn254Fq::from(2u64),
        Bn254Fq::from(3u64),
        Bn254Fq::from(5u64),
        Bn254Fq::from(7u64),
    ];
    let blind = Bn254Fq::from(11u64);
    let claimed =
        g * scalars[0] + g2_pt * scalars[1] + g3 * scalars[2] + g5 * scalars[3] + h * blind;
    let circuit_ab = RecursionDeciderCircuit::section_a_with_b_interface(
        scalars,
        bases,
        blind,
        h_aff,
        claimed,
        section_b_pis.clone(),
    );

    eprintln!("recursion-decider-b-fixture-emit: prove …");
    let proof = prove_recursion_decider(&pk, circuit_ab, &mut rng).expect("prove");
    let proof_bytes = proof_to_eip197_bytes(&proof);

    // ── 6. Emit JSON fixture ────────────────────────────────────────
    let pis = section_b_public_inputs_slice(&section_b_pis);
    assert_eq!(pis.len(), 11, "PI slice must be 11 elements");

    let pi_arr: Vec<String> = pis.iter().map(|f| hex(&fr_be32(f))).collect();
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

    let out_path = "ethereum-bridge/contracts/fixtures/recursion_decider_b_smoke.json";
    std::fs::write(out_path, serde_json::to_string_pretty(&json).unwrap()).expect("write fixture");
    eprintln!(
        "recursion-decider-b-fixture-emit: wrote {out_path} \
         (proof={}, pis={}, ic={})",
        proof_bytes.len(),
        pis.len(),
        vk.gamma_abc_g1.len()
    );
}
