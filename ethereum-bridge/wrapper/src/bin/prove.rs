//! `wrapper-prove` — CLI for the Groth16 wrapper (sub-B starter).
//!
//! Reads a `verkle_proof_v2_sample.json` fixture (emitted by
//! `verkle-fixture-emit` in the sister `circuits/` workspace), runs
//! Groth16 setup + prove against the [`WrapperCircuit`] starter, and
//! writes:
//!
//!   - `--out <stem>.proof.bin`  — 256-byte Groth16 proof
//!   - `--out <stem>.vk.bin`     — Groth16 verifying key (CanonicalSerialize)
//!
//! ## Build
//!
//! ```bash
//! cd ethereum-bridge/wrapper
//! cargo build --release --bin wrapper-prove
//! ```
//!
//! ## Run
//!
//! ```bash
//! ./target/release/wrapper-prove \
//!   --fixture ../contracts/fixtures/verkle_proof_v2_sample.json \
//!   --out ./out/wrapper
//! ```
//!
//! ## Safety
//!
//! Uses the **unsafe** in-process trusted-setup from
//! [`evaporchain_verkle_wrapper::setup`]. Do not deploy the emitted
//! VK to L1. Sub-C ships the real ceremony.

use ark_serialize::CanonicalSerialize;
use ark_std::rand::SeedableRng;
use evaporchain_verkle_wrapper::{prove, proof_bytes_to_eip197, setup, verify, VerkleFixture};
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut fixture_path = PathBuf::new();
    let mut out_stem = PathBuf::from("./wrapper");

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--fixture" => {
                fixture_path = PathBuf::from(args.get(i + 1).expect("--fixture requires path"));
                i += 2;
            }
            "--out" => {
                out_stem = PathBuf::from(args.get(i + 1).expect("--out requires stem path"));
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

    if fixture_path.as_os_str().is_empty() {
        eprintln!("--fixture <path> is required");
        print_usage();
        std::process::exit(2);
    }

    eprintln!("== wrapper-prove ==");
    eprintln!("  fixture = {}", fixture_path.display());
    eprintln!("  out     = {}.{{proof,vk}}.bin", out_stem.display());

    eprintln!("Loading fixture...");
    let fixture = VerkleFixture::from_path(&fixture_path)
        .unwrap_or_else(|e| panic!("Cannot load fixture: {e}"));
    eprintln!(
        "  k = {}, halo2 proof = {} bytes",
        fixture.k,
        fixture.halo2_ipa_proof_bytes.len()
    );

    // Deterministic seed so re-runs produce identical proofs against a
    // fresh setup. Sub-C will use real ceremony output; this is fine
    // for the starter pipeline.
    let mut rng = ark_std::rand::rngs::StdRng::seed_from_u64(0xC0FFEE_u64);

    eprintln!("Running unsafe in-process trusted-setup...");
    let (pk, vk) = setup(&mut rng).expect("setup must succeed");

    eprintln!("Generating Groth16 proof against starter circuit...");
    let proof_bytes = prove(
        &pk,
        fixture.public_inputs.clone(),
        fixture.halo2_ipa_proof_bytes.clone(),
        &mut rng,
    )
    .expect("prove must succeed");
    eprintln!("  proof = {} bytes", proof_bytes.len());

    eprintln!("Self-verifying...");
    verify(&vk, &fixture.public_inputs, &proof_bytes).expect("verify must succeed");

    eprintln!("Converting to EIP-197 calldata (256-byte uncompressed)...");
    let eip197_bytes =
        proof_bytes_to_eip197(&proof_bytes).expect("eip197 conversion must succeed");
    eprintln!("  eip197 = {} bytes", eip197_bytes.len());

    let proof_path = with_suffix(&out_stem, ".proof.bin");
    let eip197_path = with_suffix(&out_stem, ".eip197.bin");
    let vk_path = with_suffix(&out_stem, ".vk.bin");
    if let Some(parent) = proof_path.parent() {
        std::fs::create_dir_all(parent).unwrap_or_else(|e| panic!("mkdir {}: {e}", parent.display()));
    }
    std::fs::write(&proof_path, &proof_bytes)
        .unwrap_or_else(|e| panic!("write {}: {e}", proof_path.display()));
    std::fs::write(&eip197_path, eip197_bytes)
        .unwrap_or_else(|e| panic!("write {}: {e}", eip197_path.display()));

    let mut vk_bytes = Vec::new();
    vk.serialize_compressed(&mut vk_bytes)
        .expect("vk serialize");
    std::fs::write(&vk_path, &vk_bytes)
        .unwrap_or_else(|e| panic!("write {}: {e}", vk_path.display()));

    eprintln!("Wrote {} ({} bytes — arkworks compressed)", proof_path.display(), proof_bytes.len());
    eprintln!("Wrote {} ({} bytes — EIP-197 calldata for L1)", eip197_path.display(), eip197_bytes.len());
    eprintln!("Wrote {} ({} bytes — verifying key)", vk_path.display(), vk_bytes.len());
    eprintln!("Done — STARTER ONLY. Do not deploy this VK to L1.");
}

fn with_suffix(stem: &PathBuf, suffix: &str) -> PathBuf {
    let mut s = stem.clone();
    let filename = s
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("wrapper")
        .to_string();
    s.set_file_name(format!("{filename}{suffix}"));
    s
}

fn print_usage() {
    eprintln!(
        "Usage: wrapper-prove --fixture <path.json> [--out <stem>]\n\n\
         Writes <stem>.proof.bin (256 B Groth16 proof) and <stem>.vk.bin (verifying key).\n\
         Defaults: --out ./wrapper"
    );
}
