//! T0.10 Path A CI gate — exit 0 on byte parity between our
//! `compress_full` output and neptune's `crc`, exit 1 with a
//! diagnostic if a single entry diverges.
//!
//! Wraps PR #103's `compress_ark::compress_full` + PR #84's
//! `extract_compressed_round_constants` into a single
//! command-line tool suitable for CI pipelines.
//!
//! # CLI
//!
//! ```bash
//! cargo run -p evaporchain-nova-bridge --bin check-neptune-parity \
//!     -- --neptune /tmp/neptune-bn256-standard.json
//! ```
//!
//! Exit codes:
//!   0  byte parity confirmed (all 259 crc entries match)
//!   1  any other failure: file missing, decode error,
//!      parity mismatch (diagnostic printed)

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use ark_ff::{BigInteger, PrimeField};
use evaporchain_nova_bridge::compress_ark::compress_full;
use evaporchain_nova_bridge::grain_lfsr::generate_round_constants_bn254_arity_24_standard;
use evaporchain_nova_bridge::neptune_dump_parser::{
    extract_compressed_round_constants, extract_mds_inverse_matrix,
};

fn main() -> ExitCode {
    let neptune_path = match parse_args(env::args().skip(1)) {
        Some(p) => p,
        None => {
            eprintln!("check-neptune-parity: usage: --neptune PATH");
            return ExitCode::from(1);
        }
    };

    match run(neptune_path) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("check-neptune-parity: FAIL: {e}");
            ExitCode::from(1)
        }
    }
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Option<PathBuf> {
    while let Some(arg) = args.next() {
        if arg == "--neptune" {
            return args.next().map(PathBuf::from);
        }
    }
    None
}

fn run(neptune_path: PathBuf) -> Result<(), String> {
    println!(
        "check-neptune-parity: loading neptune dump from {} …",
        neptune_path.display()
    );
    let m_inv = extract_mds_inverse_matrix(&neptune_path)
        .map_err(|e| format!("extract m_inv: {e}"))?;
    let neptune_crc = extract_compressed_round_constants(&neptune_path)
        .map_err(|e| format!("extract crc: {e}"))?;

    println!("check-neptune-parity: generating our compressed ARK …");
    let plain_ark = generate_round_constants_bn254_arity_24_standard();
    let ours = compress_full(&plain_ark, &m_inv, 25, 8, 59);

    if ours.len() != neptune_crc.len() {
        return Err(format!(
            "length mismatch: ours={} neptune={}",
            ours.len(),
            neptune_crc.len()
        ));
    }

    let mut mismatches: Vec<usize> = Vec::new();
    for i in 0..ours.len() {
        if ours[i] != neptune_crc[i] {
            mismatches.push(i);
        }
    }

    if !mismatches.is_empty() {
        eprintln!(
            "check-neptune-parity: {} of {} entries differ",
            mismatches.len(),
            ours.len()
        );
        eprintln!(
            "  first 10 mismatch indices: {:?}",
            &mismatches[..mismatches.len().min(10)]
        );
        let i = mismatches[0];
        let ours_le = ours[i].into_bigint().to_bytes_le();
        let theirs_le = neptune_crc[i].into_bigint().to_bytes_le();
        eprintln!("  ours[{i}]    LE: {ours_le:?}");
        eprintln!("  theirs[{i}]  LE: {theirs_le:?}");
        return Err(format!("byte parity FAILED on {} entries", mismatches.len()));
    }

    println!(
        "check-neptune-parity: PASS — {} of {} crc entries match byte-for-byte",
        ours.len(),
        ours.len()
    );
    Ok(())
}
