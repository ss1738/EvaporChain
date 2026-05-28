//! Operator binary that runs the **real-fixture-witness** proof
//! pipeline end-to-end and prints the 256-byte EIP-197 wire-
//! format proof as hex.
//!
//! Unlike its sibling `dummy-proof-emit` (#149), this binary
//! exercises every wrapper with real values:
//!
//! ```text
//! generate_fixture(num_steps)
//!   → extract_committed_hashes_via_serde      (#151)
//!   → scalar_adapter                          (#143)
//!   → build_circuit_from_fixture              (#152)
//!   → groth16_wrapper::setup                  (#145)
//!   → groth16_wrapper::prove                  (#146)
//!   → proof_to_eip197_bytes                   (#147)
//! ```
//!
//! # Usage
//!
//! ```text
//! fixture-proof-emit [--steps <usize>] [--vk-out <path>] [--public-inputs-out <path>]
//! ```
//!
//! - `--steps`              (default 2) number of Nova fold steps.
//! - `--vk-out`             (optional)  compressed-vk bytes path.
//! - `--public-inputs-out`  (optional)  newline-separated hex
//!   strings of the public inputs (in the order Section 1 pins:
//!   committed_hash_primary, committed_hash_secondary, z0[..],
//!   zi[..]). The L1 verifier needs this slice alongside the
//!   proof bytes.
//!
//! # Output
//!
//! - Stdout: one line of 512 hex chars (256-byte proof, paste-
//!   ready into a Solidity test harness).
//! - Stderr: timing diagnostics + file-output paths.
//!
//! # Status caveat
//!
//! The proof now BINDS to a specific Nova accumulator state
//! (the committed hashes flow from the real fixture). Sections
//! Sections 2+3 are wired and gated on `section2/3.is_some()`.
//! This binary uses `build_circuit_from_fixture` without attaching
//! section witnesses, so only Section 1 is active. Use
//! `build_circuit_with_section2/3` for full verification.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use ark_ff::{BigInteger, PrimeField};
use ark_serialize::CanonicalSerialize;
use ark_std::rand::SeedableRng;

use evaporchain_nova_bridge::circuit_builder::build_circuit_from_fixture;
use evaporchain_nova_bridge::eip197::{proof_to_eip197_bytes, EIP197_PROOF_BYTES};
use evaporchain_nova_bridge::groth16_wrapper::{prove, public_inputs_for, setup};
use evaporchain_nova_bridge::recursive_snark_fixture::generate_fixture;

fn main() -> ExitCode {
    let mut steps: usize = 2;
    let mut seed: u64 = 0;
    let mut vk_out: Option<PathBuf> = None;
    let mut pi_out: Option<PathBuf> = None;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--steps" => {
                steps = parse_arg(&mut args, "--steps");
            }
            "--seed" => {
                seed = parse_arg(&mut args, "--seed");
            }
            "--vk-out" => {
                vk_out = Some(PathBuf::from(parse_arg::<String>(&mut args, "--vk-out")));
            }
            "--public-inputs-out" => {
                pi_out = Some(PathBuf::from(parse_arg::<String>(
                    &mut args,
                    "--public-inputs-out",
                )));
            }
            "-h" | "--help" => {
                eprintln!(
                    "fixture-proof-emit [--steps <usize>] [--seed <u64>] \
                     [--vk-out <path>] [--public-inputs-out <path>]"
                );
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("fixture-proof-emit: unknown argument `{other}`");
                return ExitCode::FAILURE;
            }
        }
    }

    eprintln!("fixture-proof-emit: steps={steps} seed={seed}");

    let t_fixture = Instant::now();
    let rs = match generate_fixture(steps) {
        Ok(rs) => rs,
        Err(e) => {
            eprintln!("fixture-proof-emit: generate_fixture failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    eprintln!(
        "fixture-proof-emit: nova fixture ({steps} steps) generated in {:.2}s",
        t_fixture.elapsed().as_secs_f64()
    );

    let circuit = match build_circuit_from_fixture(&rs) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("fixture-proof-emit: build_circuit_from_fixture failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    eprintln!(
        "fixture-proof-emit: real witness — committed_hash_primary={} committed_hash_secondary={}",
        fr_to_hex_be(&circuit.committed_hash_primary),
        fr_to_hex_be(&circuit.committed_hash_secondary),
    );
    eprintln!("fixture-proof-emit: zi[0]={}", fr_to_hex_be(&circuit.zi[0]));

    let t_setup = Instant::now();
    let mut rng = ark_std::rand::rngs::StdRng::seed_from_u64(seed);
    let (pk, vk) = match setup(&mut rng) {
        Ok(keys) => keys,
        Err(e) => {
            eprintln!("fixture-proof-emit: setup failed: {e:?}");
            return ExitCode::FAILURE;
        }
    };
    eprintln!(
        "fixture-proof-emit: setup done in {:.2}s",
        t_setup.elapsed().as_secs_f64()
    );

    if let Some(ref path) = vk_out {
        let mut bytes = Vec::new();
        if let Err(e) = vk.serialize_compressed(&mut bytes) {
            eprintln!("fixture-proof-emit: vk serialize failed: {e}");
            return ExitCode::FAILURE;
        }
        if let Err(e) = fs::write(path, &bytes) {
            eprintln!("fixture-proof-emit: vk write failed: {e}");
            return ExitCode::FAILURE;
        }
        eprintln!(
            "fixture-proof-emit: wrote vk ({} bytes) to {}",
            bytes.len(),
            path.display()
        );
    }

    if let Some(ref path) = pi_out {
        let mut buf = String::new();
        for input in public_inputs_for(&circuit) {
            buf.push_str(&fr_to_hex_be(&input));
            buf.push('\n');
        }
        if let Err(e) = fs::write(path, &buf) {
            eprintln!("fixture-proof-emit: public-inputs write failed: {e}");
            return ExitCode::FAILURE;
        }
        eprintln!(
            "fixture-proof-emit: wrote public inputs to {}",
            path.display()
        );
    }

    let t_prove = Instant::now();
    let proof = match prove(&pk, circuit, &mut rng) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("fixture-proof-emit: prove failed: {e:?}");
            return ExitCode::FAILURE;
        }
    };
    eprintln!(
        "fixture-proof-emit: prove done in {:.2}s",
        t_prove.elapsed().as_secs_f64()
    );

    let bytes = proof_to_eip197_bytes(&proof);
    assert_eq!(bytes.len(), EIP197_PROOF_BYTES);

    let mut hex = String::with_capacity(2 * EIP197_PROOF_BYTES);
    for byte in &bytes {
        use std::fmt::Write;
        write!(&mut hex, "{byte:02x}").expect("write");
    }
    println!("{hex}");
    eprintln!(
        "fixture-proof-emit: emitted {EIP197_PROOF_BYTES} bytes ({} hex chars)",
        hex.len()
    );

    ExitCode::SUCCESS
}

fn parse_arg<T: std::str::FromStr>(args: &mut impl Iterator<Item = String>, flag: &str) -> T
where
    T::Err: std::fmt::Display,
{
    let v = args.next().unwrap_or_else(|| {
        eprintln!("fixture-proof-emit: {flag} needs an argument");
        std::process::exit(1);
    });
    v.parse().unwrap_or_else(|e| {
        eprintln!("fixture-proof-emit: cannot parse {flag} `{v}`: {e}");
        std::process::exit(1);
    })
}

fn fr_to_hex_be(f: &ark_bn254::Fr) -> String {
    let mut le = f.into_bigint().to_bytes_le();
    le.resize(32, 0);
    le.reverse();
    let mut s = String::with_capacity(64);
    for byte in &le {
        use std::fmt::Write;
        write!(&mut s, "{byte:02x}").expect("write");
    }
    s
}
