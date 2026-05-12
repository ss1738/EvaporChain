//! T0.10 Path A operator binary — run trusted setup for the
//! Nova-bridge Groth16 circuit and persist the resulting
//! `pk` + `vk` to disk.
//!
//! # Usage
//!
//! ```bash
//! cargo run -p evaporchain-nova-bridge --bin setup-keys -- \
//!     --out-dir ./bridge-keys
//! ```
//!
//! Writes:
//! - `<out-dir>/pk.bin` — proving key, canonical compressed bytes
//! - `<out-dir>/vk.bin` — verifying key, canonical compressed bytes
//!
//! Prints to stdout:
//! - Constraint count / public-input arity at setup time
//! - File sizes
//! - Sanity check: re-load both keys and verify a freshly-generated
//!   proof to confirm the on-disk artifacts are usable
//!
//! # Caveats
//!
//! - Uses `OsRng` for the trusted-setup randomness. This is the
//!   right thing for a SINGLE-PARTY setup (development, testing,
//!   one-trustee deployments). For mainnet, replace with a
//!   multi-participant BGM17 / Phase-2 ceremony output.
//! - Setup runs against `NovaVerifierCircuit::dummy()` so the
//!   shape is fixed at arity 1. If the chain's `RealBlockCircuit`
//!   has a different arity (currently 8 per
//!   `evaporchain-proving/src/nova.rs`), the dummy needs updating
//!   before this binary produces compatible keys.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use ark_bn254::Fr;
use evaporchain_nova_bridge::canonical_io::{pk_to_bytes, vk_from_bytes, vk_to_bytes};
use rand::rngs::OsRng;
use evaporchain_nova_bridge::groth16_wrapper::{
    prove, public_inputs_in_alloc_order, setup, verify,
};
use evaporchain_nova_bridge::verifier_circuit::NovaVerifierCircuit;

fn main() -> ExitCode {
    let out_dir = parse_out_dir(env::args().skip(1));
    match run(out_dir) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("setup-keys: {e}");
            ExitCode::FAILURE
        }
    }
}

fn parse_out_dir(mut args: impl Iterator<Item = String>) -> PathBuf {
    let mut out = PathBuf::from("./bridge-keys");
    while let Some(arg) = args.next() {
        if arg == "--out-dir" {
            if let Some(val) = args.next() {
                out = PathBuf::from(val);
            }
        }
    }
    out
}

fn run(out_dir: PathBuf) -> Result<(), String> {
    fs::create_dir_all(&out_dir)
        .map_err(|e| format!("create_dir_all({}): {e}", out_dir.display()))?;

    println!("setup-keys: running trusted setup over NovaVerifierCircuit::dummy() …");
    let mut rng = OsRng;
    let keys = setup(&mut rng).map_err(|e| format!("setup: {e:?}"))?;
    let public_input_count =
        public_inputs_in_alloc_order(&NovaVerifierCircuit::dummy()).len();
    println!(
        "setup-keys: setup complete — vk.gamma_abc_g1.len()={} public_inputs={}",
        keys.vk.gamma_abc_g1.len(),
        public_input_count
    );

    let pk_bytes = pk_to_bytes(&keys.pk).map_err(|e| format!("pk_to_bytes: {e:?}"))?;
    let vk_bytes = vk_to_bytes(&keys.vk).map_err(|e| format!("vk_to_bytes: {e:?}"))?;

    let pk_path = out_dir.join("pk.bin");
    let vk_path = out_dir.join("vk.bin");
    fs::write(&pk_path, &pk_bytes)
        .map_err(|e| format!("write {}: {e}", pk_path.display()))?;
    fs::write(&vk_path, &vk_bytes)
        .map_err(|e| format!("write {}: {e}", vk_path.display()))?;

    println!(
        "setup-keys: wrote {} ({} bytes)",
        pk_path.display(),
        pk_bytes.len()
    );
    println!(
        "setup-keys: wrote {} ({} bytes)",
        vk_path.display(),
        vk_bytes.len()
    );

    // Sanity check: reload vk from disk and verify a fresh proof.
    let vk_on_disk = fs::read(&vk_path)
        .map_err(|e| format!("read {}: {e}", vk_path.display()))?;
    let vk_back = vk_from_bytes(&vk_on_disk)
        .map_err(|e| format!("vk_from_bytes: {e:?}"))?;

    let circuit = NovaVerifierCircuit::new(
        1,
        vec![Fr::from(1u64)],
        vec![Fr::from(2u64)],
        Fr::from(0xdead_beef_u64),
        Fr::from(0xcafe_babe_u64),
    );
    let public_inputs = public_inputs_in_alloc_order(&circuit);
    let proof =
        prove(&keys.pk, circuit, &mut rng).map_err(|e| format!("smoke prove: {e:?}"))?;
    let ok = verify(&vk_back, &public_inputs, &proof)
        .map_err(|e| format!("smoke verify: {e:?}"))?;
    if !ok {
        return Err("smoke-test verify returned false — on-disk vk does not match the pk that produced the proof".to_string());
    }

    println!("setup-keys: smoke-test verify passed against on-disk vk");
    println!("setup-keys: done.");
    Ok(())
}
