//! `verkle-fixture-emit` — emit a real `verkle_proof_v2_sample.json`
//! fixture for the Solidity verifier (T0.10).
//!
//! Replaces the starter placeholder fixture at
//! `ethereum-bridge/contracts/fixtures/verkle_proof_v2_sample.json` with
//! one whose inner `verkle_proof_v2.proof_bytes_hex` is real Halo2 IPA
//! bytes produced by [`VerkleProverV2::prove_v2`]. The top-level
//! `groth16_proof` field remains 256 zero bytes until T0.10 sub-B
//! (Halo2-IPA-in-BN254 wrapper circuit) + sub-C (trusted-setup ceremony)
//! land — at which point a follow-up emitter will fill it.
//!
//! ## Build
//!
//! ```bash
//! cd ethereum-bridge/circuits
//! cargo build --release --features v2-ecc --bin verkle-fixture-emit
//! ```
//!
//! ## Run
//!
//! ```bash
//! ./target/release/verkle-fixture-emit \
//!   --out ../contracts/fixtures/verkle_proof_v2_sample.json \
//!   --k 11 --path-index 7
//! ```
//!
//! Defaults match the starter fixture (k=11, path_index=7), and the
//! output path is resolved relative to `CARGO_MANIFEST_DIR` so calling
//! `cargo run --features v2-ecc --bin verkle-fixture-emit` from any
//! directory produces the same file.

use evaporchain_verkle_circuits::circuit_v2::{
    EccVerkleStepWitness, VerkleProverV2,
};
use halo2_proofs::pasta::pallas as halo2_pallas;
use serde_json::json;
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let default_out: PathBuf = PathBuf::from(format!(
        "{}/../contracts/fixtures/verkle_proof_v2_sample.json",
        env!("CARGO_MANIFEST_DIR")
    ));

    let mut out_path = default_out.clone();
    let mut k: u32 = 11;
    let mut path_index: u8 = 7;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--out" => {
                out_path = PathBuf::from(args.get(i + 1).expect("--out requires path"));
                i += 2;
            }
            "--k" => {
                k = args
                    .get(i + 1)
                    .expect("--k requires u32")
                    .parse()
                    .expect("--k must parse as u32");
                i += 2;
            }
            "--path-index" => {
                path_index = args
                    .get(i + 1)
                    .expect("--path-index requires u8")
                    .parse()
                    .expect("--path-index must parse as u8");
                i += 2;
            }
            "--help" | "-h" => {
                print_usage();
                std::process::exit(0);
            }
            other => {
                eprintln!("Unknown arg: {other}");
                print_usage();
                std::process::exit(2);
            }
        }
    }

    eprintln!("== verkle-fixture-emit ==");
    eprintln!("  k           = {k}");
    eprintln!("  path_index  = {path_index}");
    eprintln!("  out         = {}", out_path.display());

    eprintln!("Setting up VerkleProverV2 (Halo2 IPA Params + keygen)...");
    let prover = VerkleProverV2::setup(k).expect("VerkleProverV2::setup failed");

    eprintln!("Generating real IPA proof for the EccVerkleStepCircuit...");
    let witness = EccVerkleStepWitness::<halo2_pallas::Base> {
        sibling_x: halo2_pallas::Base::from(0u64),
        sibling_y: halo2_pallas::Base::from(0u64),
        path_index: halo2_pallas::Base::from(path_index as u64),
    };
    let proof = prover
        .prove_v2(witness)
        .expect("VerkleProverV2::prove_v2 failed");

    eprintln!("Verifying the generated proof before writing the fixture...");
    prover
        .verify_v2(&proof)
        .expect("VerkleProverV2::verify_v2 failed on freshly-produced proof");

    // The on-chain verifier consumes 4 public-input anchors as BN254 Fr
    // elements (encoded as bytes32). The scaffold circuit doesn't bind
    // them yet (sub-B does), but the fixture must already carry them so
    // the Solidity side can pin its calldata shape. Derive them
    // deterministically and mask the top byte for BN254 Fr safety.
    let state_root = derive_bn254_anchor(b"state_root");
    let key = derive_bn254_anchor(b"key");
    let value_commitment = derive_bn254_anchor(b"value_commitment");

    // Top-level params_fingerprint must match
    // verkle_proof_v2.params_fingerprint_hex so a future consumer can
    // cross-reference. Both come from the same prover instance.
    let params_fingerprint_bytes = hex::decode(&proof.params_fingerprint_hex)
        .expect("params_fingerprint_hex must hex-decode");
    assert_eq!(
        params_fingerprint_bytes.len(),
        32,
        "params_fingerprint must be 32 bytes (got {})",
        params_fingerprint_bytes.len()
    );
    let mut params_fingerprint_bytes32 = [0u8; 32];
    params_fingerprint_bytes32.copy_from_slice(&params_fingerprint_bytes);
    // Mask top byte so the value lives in BN254 Fr (conservative bound).
    params_fingerprint_bytes32[0] &= 0x2F;
    let params_fingerprint_hex_masked = hex::encode(params_fingerprint_bytes32);

    let fixture = json!({
        "_comment": format!(
            "Emitted by verkle-fixture-emit (T0.10 sub-A-finish). \
             Inner verkle_proof_v2.proof_bytes_hex is a real Halo2 IPA \
             proof from VerkleProverV2 at k={k}, path_index={path_index}. \
             The four public-input anchors are deterministically derived \
             via blake3('verkle-v2-fixture-anchor-v1' || tag) and masked \
             into BN254 Fr range. groth16_proof remains 256 zero bytes \
             until T0.10 sub-B (wrapper circuit) + sub-C (ceremony). \
             Source: ethereum-bridge/circuits/src/bin/fixture_emit.rs"
        ),
        "state_root":         format!("0x{}", hex::encode(state_root)),
        "key":                format!("0x{}", hex::encode(key)),
        "value_commitment":   format!("0x{}", hex::encode(value_commitment)),
        "params_fingerprint": format!("0x{}", params_fingerprint_hex_masked),
        "groth16_proof":      format!("0x{}", "00".repeat(256)),
        "verkle_proof_v2": {
            "_schema_version":         1,
            "_source": format!(
                "verkle-fixture-emit (k={k}, path_index={path_index}) — \
                 ethereum-bridge/circuits/src/bin/fixture_emit.rs"
            ),
            "proof_bytes_hex":         proof.proof_bytes_hex,
            "public_inputs":           proof.public_inputs,
            "k":                       proof.k,
            // Inner fingerprint stays unmasked (it's an internal verifier
            // commitment, not consumed as a BN254 Fr element by Solidity).
            "params_fingerprint_hex":  proof.params_fingerprint_hex,
        }
    });

    let pretty = serde_json::to_string_pretty(&fixture).expect("JSON serialize");
    std::fs::write(&out_path, pretty)
        .unwrap_or_else(|e| panic!("Cannot write {}: {e}", out_path.display()));

    eprintln!("Wrote {} ({} bytes)", out_path.display(), {
        let md = std::fs::metadata(&out_path).expect("stat");
        md.len()
    });
}

fn derive_bn254_anchor(tag: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"verkle-v2-fixture-anchor-v1");
    hasher.update(tag);
    let mut out: [u8; 32] = *hasher.finalize().as_bytes();
    // BN254 Fr's top byte is 0x30 — mask to 0x2F to guarantee Fr range
    // without needing a full modular reduction step.
    out[0] &= 0x2F;
    out
}

fn print_usage() {
    eprintln!(
        "Usage: verkle-fixture-emit [--out <path>] [--k <u32>] [--path-index <u8>]\n\n\
         Build: cargo build --release --features v2-ecc --bin verkle-fixture-emit\n\
         Run:   cargo run --release --features v2-ecc --bin verkle-fixture-emit\n\n\
         Defaults: out = <CARGO_MANIFEST_DIR>/../contracts/fixtures/verkle_proof_v2_sample.json\n\
                   k = 11, path-index = 7"
    );
}
