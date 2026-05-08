//! `verkle-prove` — CLI prover for Verkle membership proofs.
//!
//! Usage:
//!   verkle-prove --key <hex32> --value <hex32> \
//!                --path <path_json> --root <hex32> \
//!                --out proof.bin
//!
//! `--path` is a JSON array of [path_index, "commitment_hex32"] pairs,
//! ordered leaf-first (deepest level first).
//!
//! Example path JSON:
//!   [[5, "aabb..."], [17, "ccdd..."], [255, "eeff..."]]

use evaporchain_verkle_circuits::prover::VerkleProver;
use std::{fs, path::PathBuf};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 9 {
        eprintln!(
            "Usage: verkle-prove --key <hex32> --value <hex32> \
             --path <path.json> --root <hex32> --out <proof.bin>"
        );
        std::process::exit(1);
    }

    let mut key_hex   = String::new();
    let mut value_hex = String::new();
    let mut path_file = String::new();
    let mut root_hex  = String::new();
    let mut out_file  = PathBuf::from("proof.bin");

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--key"   => { key_hex   = args[i+1].clone(); i += 2; }
            "--value" => { value_hex = args[i+1].clone(); i += 2; }
            "--path"  => { path_file = args[i+1].clone(); i += 2; }
            "--root"  => { root_hex  = args[i+1].clone(); i += 2; }
            "--out"   => { out_file  = PathBuf::from(&args[i+1]); i += 2; }
            _         => { i += 1; }
        }
    }

    let key   = decode_hex32(&key_hex,   "--key");
    let value = decode_hex32(&value_hex, "--value");
    let root  = decode_hex32(&root_hex,  "--root");

    let path_json = fs::read_to_string(&path_file)
        .unwrap_or_else(|e| panic!("Cannot read {path_file}: {e}"));
    let path_raw: Vec<(u8, String)> = serde_json::from_str(&path_json)
        .unwrap_or_else(|e| panic!("Invalid path JSON: {e}"));

    let path: Vec<(u8, [u8; 32])> = path_raw
        .iter()
        .map(|(idx, hex)| (*idx, decode_hex32(hex, "path entry")))
        .collect();

    println!("Setting up public parameters (this takes ~5–15 s)...");
    let prover = VerkleProver::setup().unwrap_or_else(|e| panic!("Setup failed: {e}"));

    println!("Proving {} Verkle levels...", path.len());
    let proof = prover
        .prove(&key, &value, &path, &root)
        .unwrap_or_else(|e| panic!("Prove failed: {e}"));

    fs::write(&out_file, &proof.proof_bytes)
        .unwrap_or_else(|e| panic!("Cannot write {}: {e}", out_file.display()));

    println!("Proof written to {} ({} bytes)", out_file.display(), proof.proof_bytes.len());
    println!("z_0    = {}", hex::encode(&proof.z_0));
    println!("z_final= {}", hex::encode(&proof.z_final));
    println!("depth  = {}", proof.num_steps);
}

fn decode_hex32(hex: &str, field: &str) -> [u8; 32] {
    let h = hex.trim_start_matches("0x");
    let bytes = hex::decode(h)
        .unwrap_or_else(|e| panic!("Invalid hex for {field}: {e}"));
    bytes.try_into().unwrap_or_else(|_| panic!("{field} must be exactly 32 bytes"))
}
