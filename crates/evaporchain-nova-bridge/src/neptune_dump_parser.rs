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
//! Two parser kinds:
//!
//! **Structural** (no scalar decode):
//!   - [`parse_dump`] → [`NeptuneDumpShape`] — dimensions, round
//!     counts, sample hex per field.
//!
//! **Decoding** (hex → `ark_bn254::Fr`):
//!   - [`decode_hex_scalar`] — single 64-char hex → Fr.
//!   - [`extract_mds_matrix`] — `mds.m` (the plain MDS).
//!   - [`extract_mds_inverse_matrix`] — `mds.m_inv`.
//!   - [`extract_mds_m_hat`] / [`extract_mds_m_hat_inv`] /
//!     [`extract_mds_m_prime`] / [`extract_mds_m_double_prime`] —
//!     the four sparse-matrix sub-matrices.
//!   - [`extract_compressed_round_constants`] — full `crc` array.
//!   - [`expected_crc_len`] — structural pin: `full_rounds × width
//!     + partial_rounds` (= 259 for arity-24 standard).

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
    let v: Value = serde_json::from_slice(&bytes).map_err(|e| format!("parse JSON: {e}"))?;

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

/// Parse a neptune dump JSON and extract the COMPRESSED round
/// constants `crc` as `Vec<ark_bn254::Fr>`.
///
/// Neptune's `crc` is the SBOX-trick-optimized form, not plain
/// per-round constants. The expected layout (inferred from
/// neptune's `preprocessing.rs`):
///
/// ```text
///   crc[0..full_rounds * width]                 — plain ARK for the
///                                                  full rounds (first
///                                                  half + last half,
///                                                  width entries each)
///   crc[full_rounds * width
///       .. full_rounds * width + partial_rounds]
///                                              — one compressed scalar
///                                                  per partial round
///                                                  (folded SBOX trick)
/// ```
///
/// For arity-24 standard strength:
///   crc.len() = (8 × 25) + 59 = 200 + 59 = 259
///
/// Verified empirically against PR #80's Mini-1 dump.
pub fn extract_compressed_round_constants<P: AsRef<Path>>(path: P) -> Result<Vec<Fr>, String> {
    let bytes =
        fs::read(path.as_ref()).map_err(|e| format!("read {}: {e}", path.as_ref().display()))?;
    let v: Value = serde_json::from_slice(&bytes).map_err(|e| format!("parse JSON: {e}"))?;

    let crc = v
        .get("crc")
        .and_then(Value::as_array)
        .ok_or("missing `crc` array")?;

    let mut out: Vec<Fr> = Vec::with_capacity(crc.len());
    for (i, cell) in crc.iter().enumerate() {
        let hex = cell
            .as_str()
            .ok_or_else(|| format!("crc[{i}] not a string"))?;
        let fr = decode_hex_scalar(hex).map_err(|e| format!("decode crc[{i}]: {e}"))?;
        out.push(fr);
    }
    Ok(out)
}

/// Predict the expected `crc` length given Poseidon parameters.
///
/// Returns `full_rounds * width + partial_rounds` per neptune's
/// SBOX-trick-optimized layout (the full-round ARK is plain;
/// partial-round ARK is compressed to one scalar per round).
pub fn expected_crc_len(full_rounds: usize, partial_rounds: usize, width: usize) -> usize {
    full_rounds
        .saturating_mul(width)
        .saturating_add(partial_rounds)
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
    let v: Value = serde_json::from_slice(&bytes).map_err(|e| format!("parse JSON: {e}"))?;

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

/// Parse a neptune dump JSON and extract the INVERSE MDS matrix
/// `mds.m_inv` as `Vec<Vec<ark_bn254::Fr>>`. Same shape as
/// [`extract_mds_matrix`] but reads the `m_inv` sub-field of
/// `mds`. Used by `compress_round_constants`-style preprocessing
/// to invert subsequent full-round ARK.
pub fn extract_mds_inverse_matrix<P: AsRef<Path>>(path: P) -> Result<Vec<Vec<Fr>>, String> {
    let bytes =
        fs::read(path.as_ref()).map_err(|e| format!("read {}: {e}", path.as_ref().display()))?;
    let v: Value = serde_json::from_slice(&bytes).map_err(|e| format!("parse JSON: {e}"))?;

    let m = v
        .get("mds")
        .and_then(|m| m.get("m_inv"))
        .and_then(Value::as_array)
        .ok_or("missing `mds.m_inv` as array")?;

    let mut out: Vec<Vec<Fr>> = Vec::with_capacity(m.len());
    for (row_idx, row) in m.iter().enumerate() {
        let cells = row
            .as_array()
            .ok_or_else(|| format!("mds.m_inv[{row_idx}] not an array"))?;
        let mut row_frs: Vec<Fr> = Vec::with_capacity(cells.len());
        for (col_idx, cell) in cells.iter().enumerate() {
            let hex = cell
                .as_str()
                .ok_or_else(|| format!("mds.m_inv[{row_idx}][{col_idx}] not a string"))?;
            let fr = decode_hex_scalar(hex)
                .map_err(|e| format!("decode mds.m_inv[{row_idx}][{col_idx}]: {e}"))?;
            row_frs.push(fr);
        }
        out.push(row_frs);
    }
    Ok(out)
}

/// Shared internal helper: parse `mds.{field}` as a 2D Fr matrix
/// from the JSON dump. Used by every per-matrix extractor.
fn extract_mds_sub_matrix<P: AsRef<Path>>(
    path: P,
    field_name: &'static str,
) -> Result<Vec<Vec<Fr>>, String> {
    let bytes =
        fs::read(path.as_ref()).map_err(|e| format!("read {}: {e}", path.as_ref().display()))?;
    let v: Value = serde_json::from_slice(&bytes).map_err(|e| format!("parse JSON: {e}"))?;
    let m = v
        .get("mds")
        .and_then(|m| m.get(field_name))
        .and_then(Value::as_array)
        .ok_or_else(|| format!("missing `mds.{field_name}` as array"))?;
    let mut out: Vec<Vec<Fr>> = Vec::with_capacity(m.len());
    for (row_idx, row) in m.iter().enumerate() {
        let cells = row
            .as_array()
            .ok_or_else(|| format!("mds.{field_name}[{row_idx}] not an array"))?;
        let mut row_frs: Vec<Fr> = Vec::with_capacity(cells.len());
        for (col_idx, cell) in cells.iter().enumerate() {
            let hex = cell
                .as_str()
                .ok_or_else(|| format!("mds.{field_name}[{row_idx}][{col_idx}] not a string"))?;
            let fr = decode_hex_scalar(hex)
                .map_err(|e| format!("decode mds.{field_name}[{row_idx}][{col_idx}]: {e}"))?;
            row_frs.push(fr);
        }
        out.push(row_frs);
    }
    Ok(out)
}

/// Extract `mds.m_hat` — the (width-1) × (width-1) MDS sub-matrix
/// used by neptune's sparse-matrix transformation. Required for
/// the eventual SBOX-trick sponge port.
pub fn extract_mds_m_hat<P: AsRef<Path>>(path: P) -> Result<Vec<Vec<Fr>>, String> {
    extract_mds_sub_matrix(path, "m_hat")
}

/// Extract `mds.m_hat_inv` — inverse of `m_hat`.
pub fn extract_mds_m_hat_inv<P: AsRef<Path>>(path: P) -> Result<Vec<Vec<Fr>>, String> {
    extract_mds_sub_matrix(path, "m_hat_inv")
}

/// Extract `mds.m_prime` — modified MDS matrix used in neptune's
/// `Poseidon::hash_optimized_static` during the partial-round
/// phase boundary.
pub fn extract_mds_m_prime<P: AsRef<Path>>(path: P) -> Result<Vec<Vec<Fr>>, String> {
    extract_mds_sub_matrix(path, "m_prime")
}

/// Extract `mds.m_double_prime` — second modified MDS matrix used
/// in neptune's sparse-matrix transformation.
pub fn extract_mds_m_double_prime<P: AsRef<Path>>(path: P) -> Result<Vec<Vec<Fr>>, String> {
    extract_mds_sub_matrix(path, "m_double_prime")
}

/// Extract the top-level `psm` (pre-sparse matrix) from a neptune
/// dump. This is the matrix neptune uses at the boundary round
/// `current_round == half_full_rounds - 1`. Shape: `width × width`
/// (25 × 25 for chain Poseidon-128 arity-24 Standard).
///
/// Used by [`crate::neptune_permutation_gadget::NeptuneParams::pre_sparse_mds`].
pub fn extract_pre_sparse_matrix<P: AsRef<Path>>(path: P) -> Result<Vec<Vec<Fr>>, String> {
    let bytes = fs::read_to_string(path.as_ref())
        .map_err(|e| format!("read {}: {}", path.as_ref().display(), e))?;
    let v: Value = serde_json::from_str(&bytes).map_err(|e| format!("json parse: {e}"))?;

    let psm = v
        .get("psm")
        .and_then(|x| x.as_array())
        .ok_or_else(|| "missing top-level `psm` array".to_string())?;

    let mut out: Vec<Vec<Fr>> = Vec::with_capacity(psm.len());
    for (i, row) in psm.iter().enumerate() {
        let row_arr = row
            .as_array()
            .ok_or_else(|| format!("psm[{i}] is not an array"))?;
        let mut row_vec: Vec<Fr> = Vec::with_capacity(row_arr.len());
        for (j, cell) in row_arr.iter().enumerate() {
            let hex = cell
                .as_str()
                .ok_or_else(|| format!("psm[{i}][{j}] is not a hex string"))?;
            row_vec.push(decode_hex_scalar(hex).map_err(|e| format!("psm[{i}][{j}]: {e}"))?);
        }
        out.push(row_vec);
    }
    Ok(out)
}

/// One sparse matrix as it appears in neptune's `sm` array. Mirrors
/// neptune's `SparseMatrix<F>` struct exactly. Compatible with
/// [`crate::neptune_permutation_gadget::NeptuneSparseMatrix`] (same
/// `w_hat` + `v_rest` shape).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSparseMatrix {
    /// First column of the sparse matrix (length = width).
    pub w_hat: Vec<Fr>,
    /// First row beyond column 0 (length = width - 1).
    pub v_rest: Vec<Fr>,
}

/// Extract the top-level `sm` (sparse matrices) array from a neptune
/// dump. One sparse matrix per partial round; length = `partial_rounds`.
///
/// For chain Poseidon-128 arity-24 Standard: 59 sparse matrices,
/// each with `w_hat: Vec<Fr>[25]` + `v_rest: Vec<Fr>[24]`.
///
/// Used by [`crate::neptune_permutation_gadget::NeptuneParams::sparse_matrices`]
/// after converting each `ParsedSparseMatrix` into a
/// `NeptuneSparseMatrix` (same shape, no field-order conversion needed).
pub fn extract_sparse_matrices<P: AsRef<Path>>(path: P) -> Result<Vec<ParsedSparseMatrix>, String> {
    let bytes = fs::read_to_string(path.as_ref())
        .map_err(|e| format!("read {}: {}", path.as_ref().display(), e))?;
    let v: Value = serde_json::from_str(&bytes).map_err(|e| format!("json parse: {e}"))?;

    let sm = v
        .get("sm")
        .and_then(|x| x.as_array())
        .ok_or_else(|| "missing top-level `sm` array".to_string())?;

    let mut out: Vec<ParsedSparseMatrix> = Vec::with_capacity(sm.len());
    for (idx, entry) in sm.iter().enumerate() {
        let obj = entry
            .as_object()
            .ok_or_else(|| format!("sm[{idx}] is not an object"))?;

        let w_hat_arr = obj
            .get("w_hat")
            .and_then(|x| x.as_array())
            .ok_or_else(|| format!("sm[{idx}] missing `w_hat` array"))?;
        let v_rest_arr = obj
            .get("v_rest")
            .and_then(|x| x.as_array())
            .ok_or_else(|| format!("sm[{idx}] missing `v_rest` array"))?;

        let mut w_hat: Vec<Fr> = Vec::with_capacity(w_hat_arr.len());
        for (i, cell) in w_hat_arr.iter().enumerate() {
            let hex = cell
                .as_str()
                .ok_or_else(|| format!("sm[{idx}].w_hat[{i}] is not a hex string"))?;
            w_hat.push(decode_hex_scalar(hex).map_err(|e| format!("sm[{idx}].w_hat[{i}]: {e}"))?);
        }

        let mut v_rest: Vec<Fr> = Vec::with_capacity(v_rest_arr.len());
        for (i, cell) in v_rest_arr.iter().enumerate() {
            let hex = cell
                .as_str()
                .ok_or_else(|| format!("sm[{idx}].v_rest[{i}] is not a hex string"))?;
            v_rest.push(decode_hex_scalar(hex).map_err(|e| format!("sm[{idx}].v_rest[{i}]: {e}"))?);
        }

        out.push(ParsedSparseMatrix { w_hat, v_rest });
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
    fn expected_crc_len_arity_24_standard() {
        // Empirically verified against PR #80's real dump.
        assert_eq!(expected_crc_len(8, 59, 25), 259);
    }

    #[test]
    fn extract_compressed_round_constants_from_fixture() {
        let dir = std::env::temp_dir();
        let path = dir.join("neptune-crc-fixture.json");
        // 3-element crc: [1, 2, 3] as LE scalars
        let one_hex = format!("01{}", "0".repeat(62));
        let two_hex = format!("02{}", "0".repeat(62));
        let three_hex = format!("03{}", "0".repeat(62));
        let json = format!(
            "{{\"mds\":{{\"m\":[[\"{one_hex}\"]],\"m_inv\":[],\"m_hat\":[],\"m_hat_inv\":[],\"m_prime\":[],\"m_double_prime\":[]}},\"crc\":[\"{one_hex}\",\"{two_hex}\",\"{three_hex}\"],\"psm\":[],\"sm\":[],\"s\":\"S\",\"ht\":\"H\",\"rf\":8,\"rp\":59}}"
        );
        fs::write(&path, json).unwrap();
        let crc = extract_compressed_round_constants(&path).expect("extract");
        assert_eq!(crc.len(), 3);
        assert_eq!(crc[0], Fr::from(1u64));
        assert_eq!(crc[1], Fr::from(2u64));
        assert_eq!(crc[2], Fr::from(3u64));
        let _ = fs::remove_file(&path);
    }

    /// Real-data: each of the 4 MDS sub-matrices extracts from
    /// PR #80's dump with non-empty shape. `m_hat` and
    /// `m_hat_inv` are (width-1)×(width-1) = 24×24 for arity-24.
    /// `m_prime` and `m_double_prime` are width×width = 25×25.
    #[test]
    #[ignore = "requires /tmp/neptune-bn256-standard.json"]
    fn all_mds_sub_matrices_extract_from_real_dump() {
        let path = "/tmp/neptune-bn256-standard.json";
        let m_hat = extract_mds_m_hat(path).expect("m_hat");
        let m_hat_inv = extract_mds_m_hat_inv(path).expect("m_hat_inv");
        let m_prime = extract_mds_m_prime(path).expect("m_prime");
        let m_dp = extract_mds_m_double_prime(path).expect("m_double_prime");

        assert_eq!(m_hat.len(), 24, "m_hat is (width-1)×(width-1) for arity-24");
        assert_eq!(m_hat_inv.len(), 24);
        assert_eq!(m_prime.len(), 25, "m_prime is width×width");
        assert_eq!(m_dp.len(), 25);
        // Spot-check inner dims.
        assert_eq!(m_hat[0].len(), 24);
        assert_eq!(m_prime[0].len(), 25);
    }

    /// Missing sub-matrix → clean `Err`.
    #[test]
    fn missing_sub_matrix_errors_cleanly() {
        let dir = std::env::temp_dir();
        let path = dir.join("neptune-no-m-hat.json");
        let one = format!("01{}", "0".repeat(62));
        let json = format!(
            "{{\"mds\":{{\"m\":[[\"{one}\"]],\"m_inv\":[],\"m_hat_inv\":[],\"m_prime\":[],\"m_double_prime\":[]}},\"crc\":[\"{one}\"],\"psm\":[],\"sm\":[],\"s\":\"S\",\"ht\":\"H\",\"rf\":8,\"rp\":59}}"
        );
        std::fs::write(&path, json).unwrap();
        let result = extract_mds_m_hat(&path);
        assert!(result.is_err(), "missing m_hat must fail");
        let err = result.unwrap_err();
        assert!(err.contains("m_hat"), "error must mention m_hat: {err}");
        let _ = std::fs::remove_file(&path);
    }

    /// Pre-sparse matrix extraction on a minimal fixture: 2×2
    /// `psm` of [[1, 2], [3, 4]].
    #[test]
    fn extract_pre_sparse_matrix_from_fixture() {
        let dir = std::env::temp_dir();
        let path = dir.join("neptune-psm-fixture.json");
        let one_hex = format!("01{}", "0".repeat(62));
        let two_hex = format!("02{}", "0".repeat(62));
        let three_hex = format!("03{}", "0".repeat(62));
        let four_hex = format!("04{}", "0".repeat(62));
        let json = format!(
            "{{\"mds\":{{\"m\":[[\"{one_hex}\"]],\"m_inv\":[],\"m_hat\":[],\"m_hat_inv\":[],\"m_prime\":[],\"m_double_prime\":[]}},\
            \"crc\":[\"{one_hex}\"],\
            \"psm\":[[\"{one_hex}\",\"{two_hex}\"],[\"{three_hex}\",\"{four_hex}\"]],\
            \"sm\":[],\"s\":\"S\",\"ht\":\"H\",\"rf\":8,\"rp\":59}}"
        );
        fs::write(&path, json).unwrap();
        let psm = extract_pre_sparse_matrix(&path).expect("extract psm");
        assert_eq!(psm.len(), 2);
        assert_eq!(psm[0].len(), 2);
        assert_eq!(psm[0][0], Fr::from(1u64));
        assert_eq!(psm[0][1], Fr::from(2u64));
        assert_eq!(psm[1][0], Fr::from(3u64));
        assert_eq!(psm[1][1], Fr::from(4u64));
        let _ = fs::remove_file(&path);
    }

    /// Sparse-matrices extraction on a minimal fixture with 2 entries.
    /// First sparse matrix: w_hat=[1,2,3], v_rest=[4,5].
    /// Second sparse matrix: w_hat=[6,7,8], v_rest=[9,10].
    #[test]
    fn extract_sparse_matrices_from_fixture() {
        let dir = std::env::temp_dir();
        let path = dir.join("neptune-sm-fixture.json");
        let h = |n: u64| format!("{:02x}{}", n, "0".repeat(62));
        let json = format!(
            "{{\"mds\":{{\"m\":[[\"{h1}\"]],\"m_inv\":[],\"m_hat\":[],\"m_hat_inv\":[],\"m_prime\":[],\"m_double_prime\":[]}},\
            \"crc\":[\"{h1}\"],\"psm\":[],\
            \"sm\":[\
              {{\"w_hat\":[\"{h1}\",\"{h2}\",\"{h3}\"],\"v_rest\":[\"{h4}\",\"{h5}\"]}},\
              {{\"w_hat\":[\"{h6}\",\"{h7}\",\"{h8}\"],\"v_rest\":[\"{h9}\",\"{h10}\"]}}\
            ],\"s\":\"S\",\"ht\":\"H\",\"rf\":8,\"rp\":59}}",
            h1 = h(1), h2 = h(2), h3 = h(3), h4 = h(4), h5 = h(5),
            h6 = h(6), h7 = h(7), h8 = h(8), h9 = h(9), h10 = h(10),
        );
        fs::write(&path, json).unwrap();
        let sm = extract_sparse_matrices(&path).expect("extract sm");
        assert_eq!(sm.len(), 2);
        assert_eq!(
            sm[0].w_hat,
            vec![Fr::from(1u64), Fr::from(2u64), Fr::from(3u64)]
        );
        assert_eq!(sm[0].v_rest, vec![Fr::from(4u64), Fr::from(5u64)]);
        assert_eq!(
            sm[1].w_hat,
            vec![Fr::from(6u64), Fr::from(7u64), Fr::from(8u64)]
        );
        assert_eq!(sm[1].v_rest, vec![Fr::from(9u64), Fr::from(10u64)]);
        let _ = fs::remove_file(&path);
    }

    /// Real-data shape pin: chain Poseidon-128 arity-24 Standard dump
    /// has psm = 25×25 and 59 sparse matrices each {w_hat:25, v_rest:24}.
    #[test]
    #[ignore = "requires /tmp/neptune-bn256-standard.json"]
    fn real_neptune_psm_and_sm_have_expected_shape() {
        let path = "/tmp/neptune-bn256-standard.json";
        let psm = extract_pre_sparse_matrix(path).expect("psm");
        assert_eq!(psm.len(), 25, "psm must be 25 rows (state width)");
        for (i, row) in psm.iter().enumerate() {
            assert_eq!(row.len(), 25, "psm row {i} must be 25 cols");
        }

        let sm = extract_sparse_matrices(path).expect("sm");
        assert_eq!(
            sm.len(),
            59,
            "must have 59 sparse matrices (= partial_rounds)"
        );
        for (i, m) in sm.iter().enumerate() {
            assert_eq!(m.w_hat.len(), 25, "sm[{i}].w_hat must be width = 25");
            assert_eq!(m.v_rest.len(), 24, "sm[{i}].v_rest must be width-1 = 24");
        }
    }

    /// Missing `psm` field → typed error, not panic.
    #[test]
    fn missing_psm_errors_cleanly() {
        let dir = std::env::temp_dir();
        let path = dir.join("neptune-no-psm.json");
        let one = format!("01{}", "0".repeat(62));
        let json = format!(
            "{{\"mds\":{{\"m\":[[\"{one}\"]],\"m_inv\":[],\"m_hat\":[],\"m_hat_inv\":[],\"m_prime\":[],\"m_double_prime\":[]}},\
            \"crc\":[\"{one}\"],\"sm\":[],\"s\":\"S\",\"ht\":\"H\",\"rf\":8,\"rp\":59}}"
        );
        fs::write(&path, json).unwrap();
        let result = extract_pre_sparse_matrix(&path);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("psm"));
        let _ = fs::remove_file(&path);
    }

    /// Missing `sm` field → typed error, not panic.
    #[test]
    fn missing_sm_errors_cleanly() {
        let dir = std::env::temp_dir();
        let path = dir.join("neptune-no-sm.json");
        let one = format!("01{}", "0".repeat(62));
        let json = format!(
            "{{\"mds\":{{\"m\":[[\"{one}\"]],\"m_inv\":[],\"m_hat\":[],\"m_hat_inv\":[],\"m_prime\":[],\"m_double_prime\":[]}},\
            \"crc\":[\"{one}\"],\"psm\":[],\"s\":\"S\",\"ht\":\"H\",\"rf\":8,\"rp\":59}}"
        );
        fs::write(&path, json).unwrap();
        let result = extract_sparse_matrices(&path);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("sm"));
        let _ = fs::remove_file(&path);
    }

    /// JSON without `rf` (full_rounds) field must reject cleanly
    /// rather than parsing with a defaulted zero.
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
