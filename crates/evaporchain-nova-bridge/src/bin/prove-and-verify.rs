//! T0.10 Path A operator binary — companion to `setup-keys`.
//!
//! Loads the on-disk `pk.bin` + `vk.bin` produced by `setup-keys`,
//! builds a [`NovaVerifierCircuit`] from CLI-supplied scalars,
//! generates a Groth16 proof, verifies it locally, and writes the
//! canonical-compressed proof bytes to `<keys-dir>/proof.bin`.
//!
//! # Usage
//!
//! ```bash
//! cargo run -p evaporchain-nova-bridge --bin prove-and-verify -- \
//!     --keys-dir ./bridge-keys
//! ```
//!
//! Optional CLI args (all integer scalars, parsed as `u64` and lifted
//! into BN254 Fr):
//! - `--num-steps N`          default `1`
//! - `--z0 V`                 default `0`
//! - `--zi V`                 default `1`
//! - `--hash-primary V`       default `0`
//! - `--hash-secondary V`     default `0`
//!
//! Behaviour:
//! - Reads `<keys-dir>/pk.bin` and `<keys-dir>/vk.bin`.
//! - Generates the proof under fresh OS randomness.
//! - Verifies locally and asserts `accept == true`.
//! - Writes `<keys-dir>/proof.bin` + prints byte counts.
//!
//! # When to use
//!
//! This binary is the operator smoke test for the whole Path A
//! off-chain pipeline (setup → persist → load → prove → verify
//! → persist). It does NOT exercise the EIP-197 wire format — for
//! that, a future binary will read `proof.bin`, convert via
//! `eip197::proof_to_eip197`, and emit the 256-byte block for the
//! Solidity verifier.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use ark_bn254::Fr;
use evaporchain_nova_bridge::canonical_io::{pk_from_bytes, proof_to_bytes, vk_from_bytes};
use evaporchain_nova_bridge::groth16_wrapper::{
    prove, public_inputs_in_alloc_order, verify,
};
use evaporchain_nova_bridge::verifier_circuit::NovaVerifierCircuit;
use rand::rngs::OsRng;

#[derive(Debug)]
struct Args {
    keys_dir: PathBuf,
    num_steps: u64,
    z0: u64,
    zi: u64,
    hash_primary: u64,
    hash_secondary: u64,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            keys_dir: PathBuf::from("./bridge-keys"),
            num_steps: 1,
            z0: 0,
            zi: 1,
            hash_primary: 0,
            hash_secondary: 0,
        }
    }
}

fn main() -> ExitCode {
    let args = parse_args(env::args().skip(1));
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("prove-and-verify: {e}");
            ExitCode::FAILURE
        }
    }
}

fn parse_args(args: impl Iterator<Item = String>) -> Args {
    let mut out = Args::default();
    let mut iter = args.peekable();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--keys-dir" => {
                if let Some(v) = iter.next() {
                    out.keys_dir = PathBuf::from(v);
                }
            }
            "--num-steps" => out.num_steps = iter.next().and_then(|s| s.parse().ok()).unwrap_or(out.num_steps),
            "--z0" => out.z0 = iter.next().and_then(|s| s.parse().ok()).unwrap_or(out.z0),
            "--zi" => out.zi = iter.next().and_then(|s| s.parse().ok()).unwrap_or(out.zi),
            "--hash-primary" => out.hash_primary = iter.next().and_then(|s| s.parse().ok()).unwrap_or(out.hash_primary),
            "--hash-secondary" => out.hash_secondary = iter.next().and_then(|s| s.parse().ok()).unwrap_or(out.hash_secondary),
            _ => {}
        }
    }
    out
}

fn run(args: Args) -> Result<(), String> {
    let pk_path = args.keys_dir.join("pk.bin");
    let vk_path = args.keys_dir.join("vk.bin");
    let proof_path = args.keys_dir.join("proof.bin");

    let pk_bytes = fs::read(&pk_path)
        .map_err(|e| format!("read {}: {e} (run setup-keys first)", pk_path.display()))?;
    let vk_bytes = fs::read(&vk_path)
        .map_err(|e| format!("read {}: {e}", vk_path.display()))?;
    let pk = pk_from_bytes(&pk_bytes).map_err(|e| format!("pk_from_bytes: {e:?}"))?;
    let vk = vk_from_bytes(&vk_bytes).map_err(|e| format!("vk_from_bytes: {e:?}"))?;
    println!(
        "prove-and-verify: loaded pk ({} bytes) + vk ({} bytes) from {}",
        pk_bytes.len(),
        vk_bytes.len(),
        args.keys_dir.display()
    );

    let circuit = NovaVerifierCircuit::new(
        args.num_steps,
        vec![Fr::from(args.z0)],
        vec![Fr::from(args.zi)],
        Fr::from(args.hash_primary),
        Fr::from(args.hash_secondary),
    );
    let public_inputs = public_inputs_in_alloc_order(&circuit);
    println!(
        "prove-and-verify: circuit num_steps={} z0={} zi={} hashes=({}, {}); {} public inputs",
        args.num_steps,
        args.z0,
        args.zi,
        args.hash_primary,
        args.hash_secondary,
        public_inputs.len()
    );

    let mut rng = OsRng;
    let proof = prove(&pk, circuit, &mut rng).map_err(|e| format!("prove: {e:?}"))?;

    let ok = verify(&vk, &public_inputs, &proof).map_err(|e| format!("verify: {e:?}"))?;
    if !ok {
        return Err("proof did not verify against loaded vk".to_string());
    }
    println!("prove-and-verify: local verify: PASS");

    let proof_bytes = proof_to_bytes(&proof).map_err(|e| format!("proof_to_bytes: {e:?}"))?;
    fs::write(&proof_path, &proof_bytes)
        .map_err(|e| format!("write {}: {e}", proof_path.display()))?;
    println!(
        "prove-and-verify: wrote {} ({} bytes, canonical compressed)",
        proof_path.display(),
        proof_bytes.len()
    );
    println!("prove-and-verify: done.");
    Ok(())
}
