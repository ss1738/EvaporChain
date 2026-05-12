//! T0.10 Path A — dump neptune's `PoseidonConstants` for the
//! Bn256/Grumpkin scalar field to JSON.
//!
//! # What this binary does
//!
//! Constructs `PoseidonConstantsCircuit::<bn256::Scalar>::default()`
//! — the exact constants nova-snark's `RecursiveSNARK::verify` uses
//! to produce `hash_primary` — and serializes them via serde to
//! JSON. Output is a single file containing:
//!
//! - `mds` — the MDS matrices struct
//! - `crc` — compressed round constants (`Vec<F>`, optimized form)
//! - `psm` — pre-sparse matrix
//! - `sm`  — sparse matrices vector
//! - `s`   — strength enum
//! - `rf`  — full rounds (usize)
//! - `rp`  — partial rounds (usize)
//! - `ht`  — hash type
//!
//! # Why
//!
//! The BESPOKE Section-2 port (PR #79) needs the actual neptune
//! constants to produce a byte-equivalent arkworks gadget. Neptune
//! exposes its `PoseidonConstants` fields as `pub(crate)` so we
//! can't read them directly — but the serde `Serialize` impl
//! writes them all out (see `nova-snark/src/frontend/gadgets/
//! poseidon/serde_impl.rs:14-32`).
//!
//! This binary is the inspection artifact: dump to JSON, then a
//! follow-up PR parses + ports the values into an arkworks
//! `PoseidonConfig`.
//!
//! # Usage
//!
//! ```bash
//! cargo run -p evaporchain-nova-bridge --bin dump-neptune-constants -- \
//!     --out ./neptune-bn256-standard.json
//! ```
//!
//! # Caveats
//!
//! - `crc` is the **compressed/optimized** round-constants vector,
//!   not the plain ARK. Porting to a standard Poseidon-128
//!   implementation (which doesn't have the SBOX-trick optimization)
//!   requires either:
//!   - Inverting the compression to recover plain ARK, OR
//!   - Reimplementing the optimization in arkworks
//!   - OR regenerating plain ARK from the same grain LFSR seed
//!     using `(F.MODULUS, full_rounds, partial_rounds, width)`
//!
//! - The MDS in `mds.m` IS the plain MDS — usable directly.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use nova_snark::provider::poseidon::PoseidonConstantsCircuit;

type PrimaryScalar = nova_snark::provider::bn256_grumpkin::bn256::Scalar;

fn main() -> ExitCode {
    let out_path = parse_out_path(env::args().skip(1));
    match run(out_path) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("dump-neptune-constants: {e}");
            ExitCode::FAILURE
        }
    }
}

fn parse_out_path(mut args: impl Iterator<Item = String>) -> PathBuf {
    let mut out = PathBuf::from("./neptune-bn256-standard.json");
    while let Some(arg) = args.next() {
        if arg == "--out" {
            if let Some(v) = args.next() {
                out = PathBuf::from(v);
            }
        }
    }
    out
}

fn run(out_path: PathBuf) -> Result<(), String> {
    println!("dump-neptune-constants: building PoseidonConstantsCircuit::<bn256::Scalar>::default() …");
    let constants = PoseidonConstantsCircuit::<PrimaryScalar>::default();
    println!("dump-neptune-constants: serializing to JSON …");
    let json = serde_json::to_string_pretty(&constants)
        .map_err(|e| format!("serde_json: {e}"))?;
    fs::write(&out_path, &json)
        .map_err(|e| format!("write {}: {e}", out_path.display()))?;
    println!(
        "dump-neptune-constants: wrote {} ({} bytes)",
        out_path.display(),
        json.len()
    );
    // Quick sanity peek into the JSON: top-level keys must match
    // the serde struct field order from nova-snark's `serde_impl.rs`.
    for key in &["\"mds\"", "\"crc\"", "\"rf\"", "\"rp\""] {
        if !json.contains(key) {
            return Err(format!(
                "JSON missing expected key {key}; neptune serde format may have changed"
            ));
        }
    }
    println!("dump-neptune-constants: JSON contains expected keys (mds, crc, rf, rp)");
    println!("dump-neptune-constants: done.");
    Ok(())
}
