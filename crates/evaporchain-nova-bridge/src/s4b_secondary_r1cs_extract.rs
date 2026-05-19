//! Audit B-1/B-2 PHASE D.2: extract the SECONDARY RelaxedR1CS
//! shape + witness from a real fixture — the memory-tractable feeder
//! for the A.1-verified D.1 gadget (`s4b_secondary_r1cs_gadget::
//! enforce_secondary_relaxed_r1cs_sat_nn`).
//!
//! Byte-identical mirror of the proven `section3_witness::
//! extract_section3_witness` (which does the PRIMARY side), with:
//! - JSON paths `r1cs_shape_secondary` / `r_W_secondary` /
//!   `r_U_secondary` (instead of `*_primary`);
//! - target field `ark_bn254::Fq` (secondary R1CS field = grumpkin
//!   scalar = BN254 Fq) via `from_le_bytes_mod_order` — EXACT
//!   (value < q), the same approach proven in `secondary_to_ark_fq`;
//! - A/B/C CSR pre-bucketed by row into `SparseRow` (= the type D.1
//!   consumes).
//!
//! D.2 (this) is decode-only / memory-tractable. D.3 (full secondary
//! RelaxedR1CS enforced in-circuit) is the scale-gate (≫16 GB, like
//! B.3b) — NOT this module.

use crate::l_u_secondary_extract::ExtractError;
use crate::s4b_secondary_r1cs_gadget::SparseRow;
use crate::section3_witness::{MAX_R1CS_NUM_CONS, MAX_R1CS_NUM_IO, MAX_R1CS_NUM_VARS};
use ark_bn254::Fq as ArkFq;
use ark_ff::PrimeField;
use nova_snark::nova::{PublicParams, RecursiveSNARK};
use crate::recursive_snark_fixture::{TrivialIncrementCircuit, E1, E2};

/// Secondary RelaxedR1CS shape + witness, in `ark_bn254::Fq`,
/// A/B/C bucketed by row for `enforce_secondary_relaxed_r1cs_sat_nn`.
pub struct SecondaryR1csWitness {
    pub w: Vec<ArkFq>,
    pub e: Vec<ArkFq>,
    pub u: ArkFq,
    pub x: [ArkFq; 2],
    pub a_rows: Vec<SparseRow>,
    pub b_rows: Vec<SparseRow>,
    pub c_rows: Vec<SparseRow>,
    pub num_cons: usize,
    pub num_vars: usize,
    pub num_io: usize,
}

fn parse_usize_vec(v: &serde_json::Value) -> Result<Vec<usize>, String> {
    v.as_array()
        .ok_or("expected JSON array (usize vec)")?
        .iter()
        .map(|e| e.as_u64().map(|n| n as usize).ok_or_else(|| "non-u64".into()))
        .collect()
}

/// 64-char LE hex (halo2curves canonical, no `0x`) → `ark_bn254::Fq`.
/// Exact (value < q). Mirrors `section3_witness::parse_le_hex_scalar`,
/// target Fq.
fn parse_fq_hex(v: &serde_json::Value) -> Result<ArkFq, String> {
    let s = v.as_str().ok_or_else(|| format!("expected string, got {v:?}"))?;
    let clean = s.trim_start_matches("0x");
    if clean.len() != 64 {
        return Err(format!("expected 64 hex chars, got {}", clean.len()));
    }
    let mut bytes = [0u8; 32];
    for i in 0..32 {
        bytes[i] = u8::from_str_radix(&clean[2 * i..2 * i + 2], 16)
            .map_err(|e| format!("byte {i}: {e}"))?;
    }
    Ok(ArkFq::from_le_bytes_mod_order(&bytes))
}

fn parse_fq_vec(v: &serde_json::Value) -> Result<Vec<ArkFq>, String> {
    v.as_array()
        .ok_or("expected JSON array")?
        .iter()
        .map(parse_fq_hex)
        .collect()
}

/// CSR `{indptr, indices, data}` → rows bucketed as `SparseRow`
/// (`Vec<(col, Fq)>` per row). Mirrors `parse_csr`, Fq + bucketed.
fn parse_csr_rows_fq(
    v: &serde_json::Value,
    num_rows: usize,
) -> Result<Vec<SparseRow>, String> {
    let indptr = parse_usize_vec(&v["indptr"])?;
    let indices = parse_usize_vec(&v["indices"])?;
    let data = parse_fq_vec(&v["data"])?;
    if indptr.len() != num_rows + 1 {
        return Err(format!(
            "indptr.len()={} expected {}",
            indptr.len(),
            num_rows + 1
        ));
    }
    let mut rows: Vec<SparseRow> = vec![Vec::new(); num_rows];
    for r in 0..num_rows {
        for j in indptr[r]..indptr[r + 1] {
            rows[r].push((indices[j], data[j]));
        }
    }
    Ok(rows)
}

/// Extract the secondary RelaxedR1CS shape + witness from a real
/// `RecursiveSNARK` + `PublicParams`. Mirror of
/// `extract_section3_witness`, secondary side.
pub fn extract_secondary_r1cs_witness(
    rs: &RecursiveSNARK<E1, E2, TrivialIncrementCircuit>,
    pp: &PublicParams<E1, E2, TrivialIncrementCircuit>,
) -> Result<SecondaryR1csWitness, ExtractError> {
    let rs_val = serde_json::to_value(rs).map_err(|e| ExtractError::Serialize(e.to_string()))?;
    let pp_val = serde_json::to_value(pp).map_err(|e| ExtractError::Serialize(e.to_string()))?;

    let rw = &rs_val["r_W_secondary"];
    let w = parse_fq_vec(&rw["W"]).map_err(|e| ExtractError::MissingField(format!("r_W_secondary.W: {e}")))?;
    let e = parse_fq_vec(&rw["E"]).map_err(|e| ExtractError::MissingField(format!("r_W_secondary.E: {e}")))?;

    let ru = &rs_val["r_U_secondary"];
    let u = parse_fq_hex(&ru["u"]).map_err(|e| ExtractError::MissingField(format!("r_U_secondary.u: {e}")))?;
    let x0 = parse_fq_hex(&ru["X"][0]).map_err(|e| ExtractError::MissingField(format!("r_U_secondary.X[0]: {e}")))?;
    let x1 = parse_fq_hex(&ru["X"][1]).map_err(|e| ExtractError::MissingField(format!("r_U_secondary.X[1]: {e}")))?;

    let shape = &pp_val["r1cs_shape_secondary"];
    let num_cons = shape["num_cons"].as_u64().ok_or_else(|| ExtractError::MissingField("num_cons".into()))? as usize;
    let num_vars = shape["num_vars"].as_u64().ok_or_else(|| ExtractError::MissingField("num_vars".into()))? as usize;
    let num_io = shape["num_io"].as_u64().ok_or_else(|| ExtractError::MissingField("num_io".into()))? as usize;
    if num_cons > MAX_R1CS_NUM_CONS {
        return Err(ExtractError::ShapeTooLarge { name: "num_cons", value: num_cons, cap: MAX_R1CS_NUM_CONS });
    }
    if num_vars > MAX_R1CS_NUM_VARS {
        return Err(ExtractError::ShapeTooLarge { name: "num_vars", value: num_vars, cap: MAX_R1CS_NUM_VARS });
    }
    if num_io > MAX_R1CS_NUM_IO {
        return Err(ExtractError::ShapeTooLarge { name: "num_io", value: num_io, cap: MAX_R1CS_NUM_IO });
    }

    let a_rows = parse_csr_rows_fq(&shape["A"], num_cons).map_err(|e| ExtractError::MissingField(format!("A: {e}")))?;
    let b_rows = parse_csr_rows_fq(&shape["B"], num_cons).map_err(|e| ExtractError::MissingField(format!("B: {e}")))?;
    let c_rows = parse_csr_rows_fq(&shape["C"], num_cons).map_err(|e| ExtractError::MissingField(format!("C: {e}")))?;

    Ok(SecondaryR1csWitness {
        w, e, u, x: [x0, x1], a_rows, b_rows, c_rows, num_cons, num_vars, num_io,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **D.2 (verifiable now)** — the secondary R1CS extractor
    /// decodes REAL fixture data into a self-consistent shape:
    /// dims > 0, `W.len()==num_vars`, `E.len()==num_cons`, A/B/C
    /// each have `num_cons` row-buckets, scalars parse. DECODE-ONLY
    /// (no constraint system) → memory-tractable. The full in-circuit
    /// secondary RelaxedR1CS is D.3 (scale-gate, ≫16 GB).
    #[test]
    #[ignore = "D.2: real Nova fixture (decode-only, no circuit; tractable on Mini 1)"]
    fn secondary_r1cs_extract_decodes_real_data() {
        use crate::recursive_snark_fixture::{
            canonical_public_params, generate_fixture_with_digest,
        };
        let pp = canonical_public_params().expect("canonical pp");
        let (rs, _d) = generate_fixture_with_digest(2).expect("fixture");
        let sw = extract_secondary_r1cs_witness(&rs, &pp).expect("extract secondary R1CS");

        assert!(sw.num_cons > 0 && sw.num_vars > 0, "dims must be positive");
        assert_eq!(sw.w.len(), sw.num_vars, "W.len() == num_vars");
        assert_eq!(sw.e.len(), sw.num_cons, "E.len() == num_cons");
        assert_eq!(sw.x.len(), 2, "X has 2 public inputs");
        assert_eq!(sw.a_rows.len(), sw.num_cons, "A has num_cons row-buckets");
        assert_eq!(sw.b_rows.len(), sw.num_cons, "B has num_cons row-buckets");
        assert_eq!(sw.c_rows.len(), sw.num_cons, "C has num_cons row-buckets");
        // Column indices in-range (z = [W, u, X[0], X[1]] ⇒ num_vars+3).
        let zlen = sw.num_vars + 1 + sw.num_io;
        for (m, rows) in [("A", &sw.a_rows), ("B", &sw.b_rows), ("C", &sw.c_rows)] {
            for (r, row) in rows.iter().enumerate() {
                for &(col, _) in row {
                    assert!(col < zlen, "{m} row {r} col {col} out of range (<{zlen})");
                }
            }
        }
    }
}
