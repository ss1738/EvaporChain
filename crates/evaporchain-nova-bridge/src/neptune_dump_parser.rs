//! Phase 2.2-section-2 BESPOKE step: parse the JSON dump produced
//! by `dump-neptune-constants` (PR #80) back into a structured
//! shape report.
//!
//! # What's parseable today
//!
//! From neptune's `serde_impl.rs` (`nova-snark-0.68/src/frontend/
//! gadgets/poseidon/serde_impl.rs`), `PoseidonConstants` serializes
//! to a JSON object with eight top-level keys:
//!
//! - `mds`  — sub-object with `m, m_inv, m_hat, m_hat_inv, m_prime,
//!            m_double_prime` matrices
//! - `crc`  — array of 64-char hex strings (compressed round constants)
//! - `psm`  — pre-sparse matrix
//! - `sm`   — sparse matrices array
//! - `s`    — strength tag
//! - `rf`   — `full_rounds` (integer)
//! - `rp`   — `partial_rounds` (integer)
//! - `ht`   — hash type tag
//!
//! Each scalar is serialized as a 64-char lowercase-hex string
//! representing the canonical-byte form of the halo2curves scalar
//! (BN254 Fr).
//!
//! # What this module ships
//!
//! `parse_dump(path) -> NeptuneDumpShape` — reads the JSON, walks
//! the structure, and returns dimensions / counts / a sample
//! scalar hex per field. Does NOT yet decode every scalar — the
//! halo2curves-to-arkworks conversion needs a hex-decode followed
//! by `Fr::from_repr_vartime`, which works but adds bulk this PR
//! doesn't need.
//!
//! # What's NOT here
//!
//! - Full scalar conversion to `ark_bn254::Fr`. Possible with the
//!   PR #66 `scalar_adapter` + hex-decode, but adds dep churn this
//!   PR doesn't need. Separate follow-up.
//! - Grain-LFSR ARK regeneration. `crc` is the COMPRESSED form;
//!   recovering plain ARK requires either the LFSR seed reproducer
//!   or inverting the optimization. That's the actual BESPOKE wedge.

use std::fs;
use std::path::Path;

use ark_bn254::Fr;
use ark_ff::PrimeField;
use serde_json::Value;

/// Structural report of a parsed neptune-dump JSON. All fields
/// are derived from the JSON without needing scalar decoding;
/// values that DO require decode are reported as their hex string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeptuneDumpShape {
    /// `full_rounds`. Expected 8 for `Strength::Standard`.
    pub full_rounds: usize,
    /// `partial_rounds`. Empirically 59 for arity-24 standard.
    pub partial_rounds: usize,
    /// MDS matrix `m` dimensions: (rows, cols). Expected (25, 25)
    /// for arity 24 (width = arity + 1).
    pub mds_m_dims: (usize, usize),
    /// Number of entries in the compressed round constants array.
    /// Empirically 259 for arity-24 standard.
    pub crc_len: usize,
    /// First MDS `m[0][0]` entry as the original 64-char hex
    /// string — useful for fixture pinning before scalar decode
    /// is wired up.
    pub mds_m00_hex: String,
    /// First `crc[0]` entry as 64-char hex.
    pub crc_0_hex: String,
}

/// Parse the JSON file at `path` into a [`NeptuneDumpShape`].
pub fn parse_dump<P: AsRef<Path>>(path: P) -> Result<NeptuneDumpShape, String> {
    let bytes =
        fs::read(path.as_ref()).map_err(|e| format!("read {}: {e}", path.as_ref().display()))?;
    let v: Value = serde_json::from_slice(&bytes)
        .map_err(|e| format!("parse JSON: {e}"))?;

    let rf = v
        .get("rf")
        .and_then(Value::as_u64)
        .ok_or("missing or non-int `rf`")?;
    let rp = v
        .get("rp")
        .and_then(Value::as_u64)
        .ok_or("missing or non-int `rp`")?;

    let mds_m = v
        .get("mds")
        .and_then(|m| m.get("m"))
        .and_then(Value::as_array)
        .ok_or("missing `mds.m` as array")?;
    let mds_m_rows = mds_m.len();
    let mds_m_cols = mds_m
        .first()
        .and_then(Value::as_array)
        .map(|r| r.len())
        .ok_or("`mds.m[0]` is not an array")?;
    let mds_m00_hex = mds_m
        .first()
        .and_then(Value::as_array)
        .and_then(|r| r.first())
        .and_then(Value::as_str)
        .ok_or("`mds.m[0][0]` not a string")?
        .to_string();

    let crc = v
        .get("crc")
        .and_then(Value::as_array)
        .ok_or("missing `crc` array")?;
    let crc_0_hex = crc
        .first()
        .and_then(Value::as_str)
        .ok_or("`crc[0]` not a string")?
        .to_string();

    Ok(NeptuneDumpShape {
        full_rounds: rf as usize,
        partial_rounds: rp as usize,
        mds_m_dims: (mds_m_rows, mds_m_cols),
        crc_len: crc.len(),
        mds_m00_hex,
        crc_0_hex,
    })
}

/// Decode a hex-encoded halo2curves canonical-bytes string into
/// `ark_bn254::Fr`.
///
/// Halo2curves serializes scalars via `to_repr()` which yields
/// 32 little-endian bytes; serde renders those as a 64-char
/// lowercase-hex string. To get an `ark_bn254::Fr` of the same
/// numeric value, decode hex and call `from_le_bytes_mod_order`.
///
/// Accepts an optional `0x` / `0X` prefix.
pub fn decode_hex_scalar(hex: &str) -> Result<Fr, String> {
    let stripped = hex
        .strip_prefix("0x")
        .or_else(|| hex.strip_prefix("0X"))
        .unwrap_or(hex);
    if stripped.len() != 64 {
        return Err(format!(
            "expected 64-char hex (32 bytes), got {} chars",
            stripped.len()
        ));
    }
    let mut bytes_le = [0u8; 32];
    for i in 0..32 {
        bytes_le[i] = u8::from_str_radix(&stripped[i * 2..i * 2 + 2], 16)
            .map_err(|e| format!("hex decode at byte {i}: {e}"))?;
    }
    Ok(Fr::from_le_bytes_mod_order(&bytes_le))
}

/// Parse a neptune dump JSON and extract the PLAIN MDS matrix `m`
/// as `Vec<Vec<ark_bn254::Fr>>`.
///
/// Returns the matrix in row-major order. For arity-24 standard
/// strength this is 25×25 (state width = 25).
///
/// **Note.** The MDS `m` field IS the plain MDS — it's the round
/// constants (`crc`) that are compressed. So this matrix is
/// directly usable in an arkworks `PoseidonConfig`. ARK is the
/// remaining BESPOKE wedge.
pub fn extract_mds_matrix<P: AsRef<Path>>(path: P) -> Result<Vec<Vec<Fr>>, String> {
    let bytes =
        fs::read(path.as_ref()).map_err(|e| format!("read {}: {e}", path.as_ref().display()))?;
    let v: Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("parse JSON: {e}"))?;

    let m = v
        .get("mds")
        .and_then(|m| m.get("m"))
        .and_then(Value::as_array)
        .ok_or("missing `mds.m` as array")?;

    let mut out: Vec<Vec<Fr>> = Vec::with_capacity(m.len());
    for (row_idx, row) in m.iter().enumerate() {
        let cells = row
            .as_array()
            .ok_or_else(|| format!("mds.m[{row_idx}] not an array"))?;
        let mut row_frs: Vec<Fr> = Vec::with_capacity(cells.len());
        for (col_idx, cell) in cells.iter().enumerate() {
            let hex = cell
                .as_str()
                .ok_or_else(|| format!("mds.m[{row_idx}][{col_idx}] not a string"))?;
            let fr = decode_hex_scalar(hex)
                .map_err(|e| format!("decode mds.m[{row_idx}][{col_idx}]: {e}"))?;
            row_frs.push(fr);
        }
        out.push(row_frs);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A minimal hand-rolled JSON fixture mirroring the shape
    /// `dump-neptune-constants` produces. Lets the parser test
    /// run without requiring an actual neptune extraction on
    /// the test runner.
    fn fixture_json() -> String {
        let mut s = String::new();
        s.push_str("{\n");
        // 3x3 MDS for the fixture (real one is 25x25; size choice
        // here is just to validate dimension reporting)
        s.push_str("\"mds\": {\n");
        s.push_str("  \"m\": [\n");
        s.push_str("    [\"aabbcc\", \"deadbe\", \"abc123\"],\n");
        s.push_str("    [\"112233\", \"445566\", \"778899\"],\n");
        s.push_str("    [\"ddeeff\", \"001122\", \"334455\"]\n");
        s.push_str("  ],\n");
        s.push_str("  \"m_inv\": [], \"m_hat\": [], \"m_hat_inv\": [], \"m_prime\": [], \"m_double_prime\": []\n");
        s.push_str("},\n");
        s.push_str("\"crc\": [\"fedcba\", \"123456\", \"789abc\"],\n");
        s.push_str("\"psm\": [], \"sm\": [], \"s\": \"Standard\", \"ht\": \"None\",\n");
        s.push_str("\"rf\": 8,\n");
        s.push_str("\"rp\": 59\n");
        s.push_str("}\n");
        s
    }

    #[test]
    fn parses_fixture_json_correctly() {
        let dir = std::env::temp_dir();
        let path = dir.join("neptune-fixture.json");
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(fixture_json().as_bytes()).unwrap();
        drop(f);

        let shape = parse_dump(&path).expect("parse");
        assert_eq!(shape.full_rounds, 8);
        assert_eq!(shape.partial_rounds, 59);
        assert_eq!(shape.mds_m_dims, (3, 3));
        assert_eq!(shape.crc_len, 3);
        assert_eq!(shape.mds_m00_hex, "aabbcc");
        assert_eq!(shape.crc_0_hex, "fedcba");

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn decode_hex_scalar_zero_one_roundtrip() {
        // 32 zero bytes → Fr::ZERO
        let zero_hex = "0".repeat(64);
        let z = decode_hex_scalar(&zero_hex).expect("decode");
        assert_eq!(z, Fr::from(0u64));

        // LE byte 0x01 followed by 31 zero bytes → Fr::ONE
        let mut one_hex = String::from("01");
        one_hex.push_str(&"0".repeat(62));
        let o = decode_hex_scalar(&one_hex).expect("decode");
        assert_eq!(o, Fr::from(1u64));
    }

    #[test]
    fn decode_hex_scalar_accepts_0x_prefix() {
        let z = decode_hex_scalar(&format!("0x{}", "0".repeat(64))).expect("decode");
        assert_eq!(z, Fr::from(0u64));
    }

    #[test]
    fn decode_hex_scalar_rejects_wrong_length() {
        let result = decode_hex_scalar("abcd");
        assert!(result.is_err(), "short hex must fail");
    }

    #[test]
    fn extract_mds_matrix_from_fixture() {
        let dir = std::env::temp_dir();
        let path = dir.join("neptune-mds-fixture.json");
        // 2×2 MDS with known hex entries. byte 0x01 LE → Fr::ONE
        // for the [0][0] entry, byte 0x02 LE → Fr(2) for [1][1].
        let one_hex = format!("01{}", "0".repeat(62));
        let two_hex = format!("02{}", "0".repeat(62));
        let json = format!(
            "{{\"mds\":{{\"m\":[[\"{one_hex}\",\"{one_hex}\"],[\"{one_hex}\",\"{two_hex}\"]],\"m_inv\":[],\"m_hat\":[],\"m_hat_inv\":[],\"m_prime\":[],\"m_double_prime\":[]}},\"crc\":[\"{one_hex}\"],\"psm\":[],\"sm\":[],\"s\":\"S\",\"ht\":\"H\",\"rf\":8,\"rp\":59}}"
        );
        fs::write(&path, json).unwrap();
        let mds = extract_mds_matrix(&path).expect("extract");
        assert_eq!(mds.len(), 2);
        assert_eq!(mds[0].len(), 2);
        assert_eq!(mds[0][0], Fr::from(1u64));
        assert_eq!(mds[1][1], Fr::from(2u64));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn rejects_missing_rf() {
        let dir = std::env::temp_dir();
        let path = dir.join("neptune-bad.json");
        let bad = "{\"mds\":{\"m\":[[]]},\"crc\":[\"\"]}".to_string();
        fs::write(&path, bad).unwrap();
        let result = parse_dump(&path);
        assert!(result.is_err(), "missing rf must fail");
        let _ = fs::remove_file(&path);
    }
}
