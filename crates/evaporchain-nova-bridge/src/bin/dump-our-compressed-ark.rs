//! T0.10 Path A operator binary — emit OUR compressed-ARK
//! (produced by `compress_full` over our LFSR-generated plain ARK
//! and neptune's extracted `m_inv`) as JSON, in the same shape
//! as PR #80's `dump-neptune-constants` `crc` field.
//!
//! Use case: differential verification. Operator runs
//!
//!   cargo run -p evaporchain-nova-bridge --bin dump-neptune-constants \
//!       -- --out /tmp/neptune.json
//!   cargo run -p evaporchain-nova-bridge --bin dump-our-compressed-ark \
//!       -- --in /tmp/neptune.json --out /tmp/ours.json
//!   diff <(jq .crc /tmp/neptune.json) <(jq .crc /tmp/ours.json)
//!
//! Empty diff = our impl reproduces neptune's compressed ARK
//! byte-for-byte (verified at PR #103).
//!
//! # CLI
//!
//! - `--in PATH`  (required): path to neptune dump JSON, read for the
//!                `mds.m_inv` matrix
//! - `--out PATH` (default `./our-compressed-ark.json`): output JSON
//!                file with a `crc` array of 64-char hex strings
//!
//! The output schema is intentionally minimal — only `crc` is
//! emitted, so this file is not a full PoseidonConstants dump.
#![allow(clippy::doc_overindented_list_items)]

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use ark_ff::{BigInteger, PrimeField};
use evaporchain_nova_bridge::compress_ark::compress_full;
use evaporchain_nova_bridge::grain_lfsr::generate_round_constants_bn254_arity_24_standard;
use evaporchain_nova_bridge::neptune_dump_parser::extract_mds_inverse_matrix;

fn main() -> ExitCode {
    let (in_path, out_path) = parse_args(env::args().skip(1));
    match run(in_path, out_path) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("dump-our-compressed-ark: {e}");
            ExitCode::FAILURE
        }
    }
}

fn parse_args(mut args: impl Iterator<Item = String>) -> (Option<PathBuf>, PathBuf) {
    let mut in_path: Option<PathBuf> = None;
    let mut out_path = PathBuf::from("./our-compressed-ark.json");
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--in" => {
                if let Some(v) = args.next() {
                    in_path = Some(PathBuf::from(v));
                }
            }
            "--out" => {
                if let Some(v) = args.next() {
                    out_path = PathBuf::from(v);
                }
            }
            _ => {}
        }
    }
    (in_path, out_path)
}

fn run(in_path: Option<PathBuf>, out_path: PathBuf) -> Result<(), String> {
    let in_path = in_path.ok_or(
        "missing required `--in PATH` (path to neptune dump JSON, see `dump-neptune-constants`)",
    )?;
    println!(
        "dump-our-compressed-ark: loading inverse MDS from {} …",
        in_path.display()
    );
    let m_inv = extract_mds_inverse_matrix(&in_path)
        .map_err(|e| format!("extract m_inv: {e}"))?;

    println!("dump-our-compressed-ark: generating plain ARK via grain LFSR …");
    let plain_ark = generate_round_constants_bn254_arity_24_standard();

    println!("dump-our-compressed-ark: compressing (compress_full) …");
    let compressed = compress_full(&plain_ark, &m_inv, 25, 8, 59);
    if compressed.len() != 259 {
        return Err(format!(
            "compressed ARK length {} ≠ expected 259",
            compressed.len()
        ));
    }

    // Emit JSON with a single `crc` array of 64-char lowercase-hex
    // strings (matches neptune's serde output for the `crc` field).
    let mut json = String::from("{\n  \"crc\": [\n");
    for (i, fr) in compressed.iter().enumerate() {
        let bigint = fr.into_bigint();
        let le = bigint.to_bytes_le();
        let mut bytes = [0u8; 32];
        let copy = le.len().min(32);
        bytes[..copy].copy_from_slice(&le[..copy]);
        json.push_str("    \"");
        for b in &bytes {
            json.push_str(&format!("{b:02x}"));
        }
        if i + 1 < compressed.len() {
            json.push_str("\",\n");
        } else {
            json.push_str("\"\n");
        }
    }
    json.push_str("  ]\n}\n");

    fs::write(&out_path, &json).map_err(|e| format!("write {}: {e}", out_path.display()))?;
    println!(
        "dump-our-compressed-ark: wrote {} ({} bytes, {} crc entries)",
        out_path.display(),
        json.len(),
        compressed.len()
    );
    println!("dump-our-compressed-ark: done.");
    println!();
    println!(
        "Verify against neptune's dump:  diff <(jq .crc {}) <(jq .crc {})",
        in_path.display(),
        out_path.display()
    );
    Ok(())
}
