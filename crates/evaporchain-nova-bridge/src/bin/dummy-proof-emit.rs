//! Operator binary that exercises the full Phase 2.4 + 2.5
//! proof-emission pipeline on a dummy witness, printing the
//! 256-byte EIP-197 wire-format proof as hex and optionally
//! serializing the verifying key for L1 deployment.
//!
//! # Usage
//!
//! ```text
//! dummy-proof-emit [--seed <u64>] [--vk-out <path>]
//! ```
//!
//! - `--seed`   (optional, default 0)      deterministic RNG seed.
//! - `--vk-out` (optional)                 if given, writes the
//!   compressed-serialized verifying key bytes to the path. Use
//!   for one-shot L1 verifier setup.
//!
//! # Output
//!
//! - One line of ~512 hex chars (256 bytes × 2) — the EIP-197
//!   proof blob, paste-ready into a Solidity test harness or
//!   `cast call` to the BN254 pairing precompile.
//! - Diagnostic lines on stderr (timing, vk path) so the stdout
//!   stays clean for shell piping.
//!
//! # Status caveat
//!
//! The proof is over `NovaVerifierCircuit::dummy()` — Sections
//! Sections 2+3 are wired in `generate_constraints` but gated
//! on `section2/3.is_some()`; dummy() omits both so this proof
//! covers the Section 1 structural gate only.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use ark_serialize::CanonicalSerialize;
use ark_std::rand::SeedableRng;

use evaporchain_nova_bridge::eip197::{proof_to_eip197_bytes, EIP197_PROOF_BYTES};
use evaporchain_nova_bridge::groth16_wrapper::{prove, setup};
use evaporchain_nova_bridge::verifier_circuit::NovaVerifierCircuit;

fn main() -> ExitCode {
    let mut seed: u64 = 0;
    let mut vk_out: Option<PathBuf> = None;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--seed" => {
                let v = args.next().unwrap_or_else(|| {
                    eprintln!("dummy-proof-emit: --seed needs a u64 argument");
                    std::process::exit(1);
                });
                seed = v.parse().unwrap_or_else(|e| {
                    eprintln!("dummy-proof-emit: cannot parse seed `{v}`: {e}");
                    std::process::exit(1);
                });
            }
            "--vk-out" => {
                let p = args.next().unwrap_or_else(|| {
                    eprintln!("dummy-proof-emit: --vk-out needs a path argument");
                    std::process::exit(1);
                });
                vk_out = Some(PathBuf::from(p));
            }
            "-h" | "--help" => {
                eprintln!("dummy-proof-emit [--seed <u64>] [--vk-out <path>]");
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("dummy-proof-emit: unknown argument `{other}`");
                return ExitCode::FAILURE;
            }
        }
    }

    eprintln!("dummy-proof-emit: seed={seed}");

    let t_setup = Instant::now();
    let mut rng = ark_std::rand::rngs::StdRng::seed_from_u64(seed);
    let (pk, vk) = match setup(&mut rng) {
        Ok(keys) => keys,
        Err(e) => {
            eprintln!("dummy-proof-emit: setup failed: {e:?}");
            return ExitCode::FAILURE;
        }
    };
    eprintln!(
        "dummy-proof-emit: setup done in {:.2}s",
        t_setup.elapsed().as_secs_f64()
    );

    if let Some(ref path) = vk_out {
        let mut vk_bytes = Vec::new();
        if let Err(e) = vk.serialize_compressed(&mut vk_bytes) {
            eprintln!("dummy-proof-emit: vk serialize failed: {e}");
            return ExitCode::FAILURE;
        }
        if let Err(e) = fs::write(path, &vk_bytes) {
            eprintln!("dummy-proof-emit: vk write failed: {e}");
            return ExitCode::FAILURE;
        }
        eprintln!(
            "dummy-proof-emit: wrote vk ({} bytes) to {}",
            vk_bytes.len(),
            path.display()
        );
    }

    let t_prove = Instant::now();
    let proof = match prove(&pk, NovaVerifierCircuit::dummy(), &mut rng) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("dummy-proof-emit: prove failed: {e:?}");
            return ExitCode::FAILURE;
        }
    };
    eprintln!(
        "dummy-proof-emit: prove done in {:.2}s",
        t_prove.elapsed().as_secs_f64()
    );

    let bytes = proof_to_eip197_bytes(&proof);
    assert_eq!(bytes.len(), EIP197_PROOF_BYTES);

    // Single line of hex on stdout — exactly 256×2 = 512 chars.
    let mut hex = String::with_capacity(2 * EIP197_PROOF_BYTES);
    for byte in &bytes {
        use std::fmt::Write;
        write!(&mut hex, "{byte:02x}").expect("write");
    }
    println!("{hex}");
    eprintln!("dummy-proof-emit: emitted {EIP197_PROOF_BYTES} bytes ({} hex chars)", hex.len());

    ExitCode::SUCCESS
}
