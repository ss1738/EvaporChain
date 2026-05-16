//! Section 3 witness: primary RelaxedR1CS satisfaction data.
//!
//! Extracts from a `RecursiveSNARK` + `PublicParams`:
//!   - `r_W_primary.W`  — primary witness vector (BN254 Fr, 64-char LE hex)
//!   - `r_W_primary.E`  — primary error vector   (BN254 Fr, 64-char LE hex)
//!   - `r_U_primary.u`  — relaxation scalar       (BN254 Fr, 64-char LE hex)
//!   - `r_U_primary.X`  — 2 public hash inputs    (BN254 Fr, 64-char LE hex)
//!   - `r1cs_shape_primary` — A, B, C sparse matrices (BN254 Fr, 64-char LE hex)
//!
//! The in-circuit check verifies for every constraint row `i`:
//!   `(Az)_i * (Bz)_i == u * (Cz)_i + E_i`
//! where `z = [W, u, X[0], X[1]]`.
//!
//! # Gaps (documented)
//!
//! - Commitment checks (`comm_W == Commit(ck, W)`, `comm_E == Commit(ck, E)`)
//!   require KZG pairing in-circuit. The Section 2 hash binds `comm_W.x/y`
//!   to the public inputs, providing partial soundness.
//! - Secondary R1CS checks (nova `is_sat_relaxed` checks 2+3) use
//!   Grumpkin Fr = BN254 Fq (non-native). Deferred.

use crate::l_u_secondary_extract::ExtractError;
use crate::recursive_snark_fixture::{TrivialIncrementCircuit, E1, E2};
use ark_bn254::Fr as ArkFr;
use ark_ff::PrimeField;
use nova_snark::nova::{PublicParams, RecursiveSNARK};

/// NCR5: maximum admitted `num_cons` in `r1cs_shape_primary`.
/// Production `TrivialIncrementCircuit` has ~10⁴ constraints; the
/// cap absorbs roughly 100× growth and refuses anything larger so
/// the gadget can't be coerced into multi-hour synthesis.
pub const MAX_R1CS_NUM_CONS: usize = 1_000_000;

/// NCR5: maximum admitted `num_vars` in `r1cs_shape_primary`.
pub const MAX_R1CS_NUM_VARS: usize = 1_000_000;

/// NCR5: maximum admitted `num_io` in `r1cs_shape_primary`. The
/// public-input arity stays small in practice (`TrivialIncrementCircuit`
/// has 4); 1 024 leaves plenty of headroom while keeping `vk_x`
/// MSM bounded.
pub const MAX_R1CS_NUM_IO: usize = 1_024;

/// Sparse matrix in COO (triplet) format with ArkFr values.
#[derive(Clone, Debug)]
pub struct SparseTriple {
    /// Number of rows in the matrix.
    pub num_rows: usize,
    /// Number of columns in the matrix.
    pub num_cols: usize,
    /// `(row, col, value)` for every non-zero entry.
    pub entries: Vec<(usize, usize, ArkFr)>,
}

/// All data required for the in-circuit primary RelaxedR1CS check.
#[derive(Clone, Debug)]
pub struct Section3Witness {
    /// `r_W_primary.W` — primary witness (native BN254 Fr)
    pub w_primary: Vec<ArkFr>,
    /// `r_W_primary.E` — primary error vector (native BN254 Fr)
    pub e_primary: Vec<ArkFr>,
    /// `r_U_primary.u` — relaxation scalar (1 at base case)
    pub u_primary: ArkFr,
    /// `r_U_primary.X` — 2 public hash inputs
    pub x_primary: [ArkFr; 2],
    /// Primary R1CS A matrix (COO)
    pub a_primary: SparseTriple,
    /// Primary R1CS B matrix (COO)
    pub b_primary: SparseTriple,
    /// Primary R1CS C matrix (COO)
    pub c_primary: SparseTriple,
    /// Number of R1CS constraints.
    pub num_cons: usize,
    /// Number of witness variables.
    pub num_vars: usize,
    /// Number of public I/O variables.
    pub num_io: usize,
}

impl Section3Witness {
    /// Augmented witness `z = [W, u, X[0], X[1]]`.
    pub fn build_z(&self) -> Vec<ArkFr> {
        let mut z = Vec::with_capacity(self.num_vars + 1 + self.num_io);
        z.extend_from_slice(&self.w_primary);
        z.push(self.u_primary);
        z.extend_from_slice(&self.x_primary);
        z
    }

    /// Off-circuit row check: `Az*Bz == u*Cz + E` for every row.
    /// Returns `Ok(())` when all rows are satisfied.
    pub fn validate_rows_native(&self) -> Result<(), String> {
        let z = self.build_z();
        let az = sparse_mv(&self.a_primary, &z);
        let bz = sparse_mv(&self.b_primary, &z);
        let cz = sparse_mv(&self.c_primary, &z);
        for i in 0..self.num_cons {
            let lhs = az[i] * bz[i];
            let rhs = self.u_primary * cz[i] + self.e_primary[i];
            if lhs != rhs {
                return Err(format!("row {i} not satisfied: {lhs:?} != {rhs:?}"));
            }
        }
        Ok(())
    }

    /// Trivially satisfied witness (zero matrices, zero vectors) for unit tests.
    #[cfg(test)]
    pub fn trivial_zero(num_cons: usize, num_vars: usize) -> Self {
        let num_io = 2;
        let num_cols = num_vars + 1 + num_io;
        Section3Witness {
            w_primary: vec![ArkFr::from(0u64); num_vars],
            e_primary: vec![ArkFr::from(0u64); num_cons],
            u_primary: ArkFr::from(1u64),
            x_primary: [ArkFr::from(0u64); 2],
            a_primary: SparseTriple { num_rows: num_cons, num_cols, entries: vec![] },
            b_primary: SparseTriple { num_rows: num_cons, num_cols, entries: vec![] },
            c_primary: SparseTriple { num_rows: num_cons, num_cols, entries: vec![] },
            num_cons,
            num_vars,
            num_io,
        }
    }
}

/// Sparse matrix-vector product `y = M * z`.
pub fn sparse_mv(m: &SparseTriple, z: &[ArkFr]) -> Vec<ArkFr> {
    let mut out = vec![ArkFr::from(0u64); m.num_rows];
    for &(row, col, ref val) in &m.entries {
        out[row] += *val * z[col];
    }
    out
}

// ── Extraction ────────────────────────────────────────────────────────────────

/// Extract Section 3 witness from a `RecursiveSNARK` + `PublicParams`.
///
/// Serialises both `rs` and `pp` to JSON.  The `pp` serialisation includes
/// the commitment key (~few MB for `TrivialIncrementCircuit`).  Acceptable
/// for a relayer binary; use the `#[ignore]` integration test to exercise.
pub fn extract_section3_witness(
    rs: &RecursiveSNARK<E1, E2, TrivialIncrementCircuit>,
    pp: &PublicParams<E1, E2, TrivialIncrementCircuit>,
) -> Result<Section3Witness, ExtractError> {
    let rs_val = serde_json::to_value(rs)
        .map_err(|e| ExtractError::SerdeError(e.to_string()))?;
    let pp_val = serde_json::to_value(pp)
        .map_err(|e| ExtractError::SerdeError(e.to_string()))?;

    // ── r_W_primary ────────────────────────────────────────────────────────
    let rw = &rs_val["r_W_primary"];
    let w_primary = parse_le_hex_vec(&rw["W"])
        .map_err(|e| ExtractError::MissingField(format!("r_W_primary.W: {e}")))?;
    let e_primary = parse_le_hex_vec(&rw["E"])
        .map_err(|e| ExtractError::MissingField(format!("r_W_primary.E: {e}")))?;

    // ── r_U_primary ────────────────────────────────────────────────────────
    let ru = &rs_val["r_U_primary"];
    let u_primary = parse_le_hex_scalar(&ru["u"])
        .map_err(|e| ExtractError::MissingField(format!("r_U_primary.u: {e}")))?;
    let x0 = parse_le_hex_scalar(&ru["X"][0])
        .map_err(|e| ExtractError::MissingField(format!("r_U_primary.X[0]: {e}")))?;
    let x1 = parse_le_hex_scalar(&ru["X"][1])
        .map_err(|e| ExtractError::MissingField(format!("r_U_primary.X[1]: {e}")))?;

    // ── r1cs_shape_primary ─────────────────────────────────────────────────
    let shape = &pp_val["r1cs_shape_primary"];
    let num_cons = shape["num_cons"].as_u64()
        .ok_or_else(|| ExtractError::MissingField("num_cons".into()))? as usize;
    let num_vars = shape["num_vars"].as_u64()
        .ok_or_else(|| ExtractError::MissingField("num_vars".into()))? as usize;
    let num_io = shape["num_io"].as_u64()
        .ok_or_else(|| ExtractError::MissingField("num_io".into()))? as usize;

    // NCR5 (re-audit 2026-05-14): cap R1CS shape parameters so a
    // malicious dump can't drive the Section 3 gadget into
    // multi-hour setup. Production `TrivialIncrementCircuit` has
    // num_cons ≈ 10 003 / num_vars ≈ 9 995; the cap is ~6×
    // expected to absorb future circuit growth without inviting
    // DoS. NCR5 pairs with the gadget's pre-bucketing fix in
    // section3_gadget.rs to turn the worst case from
    // O(num_cons × entries) into O(num_cons + entries).
    if num_cons > MAX_R1CS_NUM_CONS {
        return Err(ExtractError::ShapeTooLarge {
            name: "num_cons",
            value: num_cons,
            cap: MAX_R1CS_NUM_CONS,
        });
    }
    if num_vars > MAX_R1CS_NUM_VARS {
        return Err(ExtractError::ShapeTooLarge {
            name: "num_vars",
            value: num_vars,
            cap: MAX_R1CS_NUM_VARS,
        });
    }
    if num_io > MAX_R1CS_NUM_IO {
        return Err(ExtractError::ShapeTooLarge {
            name: "num_io",
            value: num_io,
            cap: MAX_R1CS_NUM_IO,
        });
    }

    let num_cols = num_vars + 1 + num_io;

    let a_primary = parse_csr(&shape["A"], num_cons, num_cols)
        .map_err(|e| ExtractError::MissingField(format!("A: {e}")))?;
    let b_primary = parse_csr(&shape["B"], num_cons, num_cols)
        .map_err(|e| ExtractError::MissingField(format!("B: {e}")))?;
    let c_primary = parse_csr(&shape["C"], num_cons, num_cols)
        .map_err(|e| ExtractError::MissingField(format!("C: {e}")))?;

    Ok(Section3Witness {
        w_primary,
        e_primary,
        u_primary,
        x_primary: [x0, x1],
        a_primary,
        b_primary,
        c_primary,
        num_cons,
        num_vars,
        num_io,
    })
}

// ── Parsing helpers ───────────────────────────────────────────────────────────

fn parse_csr(v: &serde_json::Value, num_rows: usize, num_cols: usize)
    -> Result<SparseTriple, String>
{
    let indptr  = parse_usize_vec(&v["indptr"])?;
    let indices = parse_usize_vec(&v["indices"])?;
    let data    = parse_le_hex_vec(&v["data"])?;
    if indptr.len() != num_rows + 1 {
        return Err(format!(
            "indptr.len()={} expected {}", indptr.len(), num_rows + 1
        ));
    }
    let mut entries = Vec::with_capacity(data.len());
    for row in 0..num_rows {
        for j in indptr[row]..indptr[row + 1] {
            entries.push((row, indices[j], data[j]));
        }
    }
    Ok(SparseTriple { num_rows, num_cols, entries })
}

fn parse_le_hex_vec(v: &serde_json::Value) -> Result<Vec<ArkFr>, String> {
    let arr = v.as_array().ok_or("expected JSON array")?;
    arr.iter().map(parse_le_hex_scalar).collect()
}

/// 64-char LE hex string (halo2curves canonical form, no `0x` prefix).
fn parse_le_hex_scalar(v: &serde_json::Value) -> Result<ArkFr, String> {
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
    Ok(ArkFr::from_le_bytes_mod_order(&bytes))
}


fn parse_usize_vec(v: &serde_json::Value) -> Result<Vec<usize>, String> {
    let arr = v.as_array().ok_or("expected JSON array for usize vec")?;
    arr.iter()
        .map(|x| {
            x.as_u64()
                .ok_or_else(|| format!("expected u64, got {x:?}"))
                .map(|n| n as usize)
        })
        .collect()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trivial_zero_rows_satisfied() {
        let s3 = Section3Witness::trivial_zero(4, 8);
        s3.validate_rows_native()
            .expect("zero-matrix trivial witness must satisfy 0=0");
    }

    #[test]
    fn build_z_length() {
        let s3 = Section3Witness::trivial_zero(2, 10);
        // z = W(10) + u(1) + X(2)
        assert_eq!(s3.build_z().len(), 13);
    }

    #[test]
    fn sparse_mv_single_entry() {
        let m = SparseTriple {
            num_rows: 2,
            num_cols: 2,
            entries: vec![(0, 1, ArkFr::from(3u64))],
        };
        let z   = vec![ArkFr::from(5u64), ArkFr::from(7u64)];
        let out = sparse_mv(&m, &z);
        assert_eq!(out[0], ArkFr::from(21u64), "3 * 7 = 21");
        assert_eq!(out[1], ArkFr::from(0u64),  "row 1 has no entries");
    }

    #[test]
    fn sparse_mv_zero_matrix_yields_zeros() {
        let m = SparseTriple { num_rows: 3, num_cols: 4, entries: vec![] };
        let z = vec![
            ArkFr::from(11u64), ArkFr::from(13u64),
            ArkFr::from(17u64), ArkFr::from(19u64),
        ];
        let out = sparse_mv(&m, &z);
        assert_eq!(out.len(), 3);
        for v in &out { assert_eq!(*v, ArkFr::from(0u64)); }
    }

    #[test]
    fn sparse_mv_multiple_entries_same_row_accumulate() {
        // Row 0: 5*z[0] + 7*z[2] = 5*10 + 7*100 = 750
        let m = SparseTriple {
            num_rows: 1, num_cols: 4,
            entries: vec![(0, 0, ArkFr::from(5u64)), (0, 2, ArkFr::from(7u64))],
        };
        let z = vec![
            ArkFr::from(10u64), ArkFr::from(999u64),
            ArkFr::from(100u64), ArkFr::from(9999u64),
        ];
        assert_eq!(sparse_mv(&m, &z)[0], ArkFr::from(750u64));
    }

    #[test]
    fn sparse_mv_dense_2x2_matches_manual_product() {
        // M = [[2,3],[5,7]] ; z = [11,13] ⇒ M*z = [61, 146]
        let m = SparseTriple {
            num_rows: 2, num_cols: 2,
            entries: vec![
                (0, 0, ArkFr::from(2u64)), (0, 1, ArkFr::from(3u64)),
                (1, 0, ArkFr::from(5u64)), (1, 1, ArkFr::from(7u64)),
            ],
        };
        let z   = vec![ArkFr::from(11u64), ArkFr::from(13u64)];
        let out = sparse_mv(&m, &z);
        assert_eq!(out[0], ArkFr::from(61u64));
        assert_eq!(out[1], ArkFr::from(146u64));
    }

    #[test]
    fn validate_rows_native_detects_unsatisfied_row() {
        // A picks W[0]=7, B picks W[1]=11, C empty. lhs=77, rhs=0.
        let s3 = Section3Witness {
            w_primary: vec![ArkFr::from(7u64), ArkFr::from(11u64)],
            e_primary: vec![ArkFr::from(0u64)],
            u_primary: ArkFr::from(1u64),
            x_primary: [ArkFr::from(0u64); 2],
            a_primary: SparseTriple {
                num_rows: 1, num_cols: 5,
                entries: vec![(0, 0, ArkFr::from(1u64))],
            },
            b_primary: SparseTriple {
                num_rows: 1, num_cols: 5,
                entries: vec![(0, 1, ArkFr::from(1u64))],
            },
            c_primary: SparseTriple { num_rows: 1, num_cols: 5, entries: vec![] },
            num_cons: 1, num_vars: 2, num_io: 2,
        };
        let err = s3.validate_rows_native().expect_err("row must fail");
        assert!(err.contains("row 0"), "error must reference failing row: {err}");
    }

    #[test]
    fn validate_rows_native_passes_on_handcrafted_satisfied_row() {
        // 3 * 4 == 1 * 12 + 0; A→W[0], B→W[1], C→W[2].
        let s3 = Section3Witness {
            w_primary: vec![ArkFr::from(3u64), ArkFr::from(4u64), ArkFr::from(12u64)],
            e_primary: vec![ArkFr::from(0u64)],
            u_primary: ArkFr::from(1u64),
            x_primary: [ArkFr::from(0u64); 2],
            a_primary: SparseTriple {
                num_rows: 1, num_cols: 6,
                entries: vec![(0, 0, ArkFr::from(1u64))],
            },
            b_primary: SparseTriple {
                num_rows: 1, num_cols: 6,
                entries: vec![(0, 1, ArkFr::from(1u64))],
            },
            c_primary: SparseTriple {
                num_rows: 1, num_cols: 6,
                entries: vec![(0, 2, ArkFr::from(1u64))],
            },
            num_cons: 1, num_vars: 3, num_io: 2,
        };
        s3.validate_rows_native().expect("3 * 4 == 1 * 12 must satisfy");
    }

    #[test]
    fn build_z_layout_is_w_then_u_then_x() {
        let s3 = Section3Witness {
            w_primary: vec![ArkFr::from(7u64), ArkFr::from(8u64)],
            e_primary: vec![],
            u_primary: ArkFr::from(99u64),
            x_primary: [ArkFr::from(11u64), ArkFr::from(12u64)],
            a_primary: SparseTriple { num_rows: 0, num_cols: 0, entries: vec![] },
            b_primary: SparseTriple { num_rows: 0, num_cols: 0, entries: vec![] },
            c_primary: SparseTriple { num_rows: 0, num_cols: 0, entries: vec![] },
            num_cons: 0, num_vars: 2, num_io: 2,
        };
        let z = s3.build_z();
        assert_eq!(z.len(), 5);
        assert_eq!(z[0], ArkFr::from(7u64));
        assert_eq!(z[1], ArkFr::from(8u64));
        assert_eq!(z[2], ArkFr::from(99u64));
        assert_eq!(z[3], ArkFr::from(11u64));
        assert_eq!(z[4], ArkFr::from(12u64));
    }

    #[test]
    fn trivial_zero_with_zero_cons_validates_vacuously() {
        let s3 = Section3Witness::trivial_zero(0, 4);
        s3.validate_rows_native()
            .expect("zero-constraint witness is vacuously satisfied");
        assert_eq!(s3.num_cons, 0);
        assert_eq!(s3.num_vars, 4);
    }

    #[test]
    fn r1cs_shape_caps_match_ncr5_doctrine() {
        // NCR5 (re-audit 2026-05-14) capped R1CS shape inputs at
        // 1_000_000 for num_cons + num_vars and 1_024 for num_io.
        // Pin so a future refactor doesn't relax the DoS gate.
        assert_eq!(MAX_R1CS_NUM_CONS, 1_000_000);
        assert_eq!(MAX_R1CS_NUM_VARS, 1_000_000);
        assert_eq!(MAX_R1CS_NUM_IO, 1_024);
    }

    /// Requires `PublicParams::setup` (~3 s) + pp JSON serialisation (~30 s).
    /// `cargo test -p evaporchain-nova-bridge section3_witness -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn extract_native_check_passes_for_real_fixture() {
        use crate::recursive_snark_fixture::generate_fixture;
        use nova_snark::nova::PublicParams;
        use nova_snark::provider::{
            hyperkzg::EvaluationEngine as EE1,
            ipa_pc::EvaluationEngine as EE2,
        };
        use nova_snark::spartan::snark::RelaxedR1CSSNARK;
        use nova_snark::traits::snark::RelaxedR1CSSNARKTrait;
        type S1 = RelaxedR1CSSNARK<E1, EE1<E1>>;
        type S2 = RelaxedR1CSSNARK<E2, EE2<E2>>;

        let circuit = TrivialIncrementCircuit;
        let pp = PublicParams::<E1, E2, TrivialIncrementCircuit>::setup(
            &circuit,
            &*S1::ck_floor(),
            &*S2::ck_floor(),
        )
        .expect("PublicParams::setup");

        let rs = generate_fixture(2).expect("2-step fixture");
        let s3 = extract_section3_witness(&rs, &pp).expect("extract_section3_witness");

        println!(
            "num_cons={} num_vars={} num_io={}",
            s3.num_cons, s3.num_vars, s3.num_io
        );
        println!(
            "W.len()={} E.len()={} A.entries={}",
            s3.w_primary.len(),
            s3.e_primary.len(),
            s3.a_primary.entries.len(),
        );

        s3.validate_rows_native()
            .expect("primary RelaxedR1CS rows must be satisfied");
        println!("Section 3 native row check: PASS");
    }

}
