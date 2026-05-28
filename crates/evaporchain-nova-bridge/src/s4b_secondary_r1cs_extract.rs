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
use crate::recursive_snark_fixture::{TrivialIncrementCircuit, E1, E2};
use crate::s4b_secondary_r1cs_gadget::SparseRow;
use crate::section3_witness::{MAX_R1CS_NUM_CONS, MAX_R1CS_NUM_IO, MAX_R1CS_NUM_VARS};
use ark_bn254::Fq as ArkFq;
use ark_ff::PrimeField;
use nova_snark::nova::{PublicParams, RecursiveSNARK};

/// Secondary RelaxedR1CS shape + witness, in `ark_bn254::Fq`,
/// A/B/C bucketed by row for `enforce_secondary_relaxed_r1cs_sat_nn`.
pub struct SecondaryR1csWitness {
    /// `r_W_secondary.W` — secondary witness vector (BN254 Fq).
    pub w: Vec<ArkFq>,
    /// `r_W_secondary.E` — secondary error vector (BN254 Fq).
    pub e: Vec<ArkFq>,
    /// `r_U_secondary.u` — relaxation scalar.
    pub u: ArkFq,
    /// `r_U_secondary.X` — 2 public-IO scalars.
    pub x: [ArkFq; 2],
    /// A matrix, rows pre-bucketed `(col, coeff)` for the D.1 gadget.
    pub a_rows: Vec<SparseRow>,
    /// B matrix, rows pre-bucketed.
    pub b_rows: Vec<SparseRow>,
    /// C matrix, rows pre-bucketed.
    pub c_rows: Vec<SparseRow>,
    /// `r1cs_shape_secondary.num_cons` (constraint-row count).
    pub num_cons: usize,
    /// `r1cs_shape_secondary.num_vars` (private-witness count).
    pub num_vars: usize,
    /// `r1cs_shape_secondary.num_io` (public-IO count).
    pub num_io: usize,
}

fn parse_usize_vec(v: &serde_json::Value) -> Result<Vec<usize>, String> {
    v.as_array()
        .ok_or("expected JSON array (usize vec)")?
        .iter()
        .map(|e| {
            e.as_u64()
                .map(|n| n as usize)
                .ok_or_else(|| "non-u64".into())
        })
        .collect()
}

/// 64-char LE hex (halo2curves canonical, no `0x`) → `ark_bn254::Fq`.
/// Exact (value < q). Mirrors `section3_witness::parse_le_hex_scalar`,
/// target Fq.
fn parse_fq_hex(v: &serde_json::Value) -> Result<ArkFq, String> {
    let s = v
        .as_str()
        .ok_or_else(|| format!("expected string, got {v:?}"))?;
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
fn parse_csr_rows_fq(v: &serde_json::Value, num_rows: usize) -> Result<Vec<SparseRow>, String> {
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
    let w = parse_fq_vec(&rw["W"])
        .map_err(|e| ExtractError::MissingField(format!("r_W_secondary.W: {e}")))?;
    let e = parse_fq_vec(&rw["E"])
        .map_err(|e| ExtractError::MissingField(format!("r_W_secondary.E: {e}")))?;

    let ru = &rs_val["r_U_secondary"];
    let u = parse_fq_hex(&ru["u"])
        .map_err(|e| ExtractError::MissingField(format!("r_U_secondary.u: {e}")))?;
    let x0 = parse_fq_hex(&ru["X"][0])
        .map_err(|e| ExtractError::MissingField(format!("r_U_secondary.X[0]: {e}")))?;
    let x1 = parse_fq_hex(&ru["X"][1])
        .map_err(|e| ExtractError::MissingField(format!("r_U_secondary.X[1]: {e}")))?;

    let shape = &pp_val["r1cs_shape_secondary"];
    let num_cons = shape["num_cons"]
        .as_u64()
        .ok_or_else(|| ExtractError::MissingField("num_cons".into()))? as usize;
    let num_vars = shape["num_vars"]
        .as_u64()
        .ok_or_else(|| ExtractError::MissingField("num_vars".into()))? as usize;
    let num_io = shape["num_io"]
        .as_u64()
        .ok_or_else(|| ExtractError::MissingField("num_io".into()))? as usize;
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

    let a_rows = parse_csr_rows_fq(&shape["A"], num_cons)
        .map_err(|e| ExtractError::MissingField(format!("A: {e}")))?;
    let b_rows = parse_csr_rows_fq(&shape["B"], num_cons)
        .map_err(|e| ExtractError::MissingField(format!("B: {e}")))?;
    let c_rows = parse_csr_rows_fq(&shape["C"], num_cons)
        .map_err(|e| ExtractError::MissingField(format!("C: {e}")))?;

    Ok(SecondaryR1csWitness {
        w,
        e,
        u,
        x: [x0, x1],
        a_rows,
        b_rows,
        c_rows,
        num_cons,
        num_vars,
        num_io,
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

    /// **D.3 — FULL secondary RelaxedR1CS satisfiability in-circuit**
    /// (THE deep one; scale-gate → satyawan-1). Composes the
    /// A.1-verified D.1 gadget (`enforce_secondary_relaxed_r1cs_sat_
    /// nn`) with the D.2-verified extractor on a REAL fixture: every
    /// `num_cons` row of the real secondary R1CS enforced non-native
    /// (`(Az)(Bz)==u(Cz)+E`), asserting the valid Nova instance IS
    /// satisfied + adversarial (perturbed `W` ⇒ a row breaks ⇒
    /// UNSATISFIABLE). `extract_secondary_r1cs_witness` does its serde
    /// internally and returns an owned struct → no JSON co-resides
    /// with the circuit (B.3 memory pattern, by construction).
    #[test]
    #[ignore = "D.3 SCALE-GATE: full secondary R1CS in-circuit; run on satyawan-1 (≫16 GB)"]
    fn secondary_r1cs_full_sat_real_data() {
        use crate::recursive_snark_fixture::{
            canonical_public_params, generate_fixture_with_digest,
        };
        use crate::s4b_secondary_r1cs_gadget::{enforce_secondary_relaxed_r1cs_sat_nn, NnFq};
        use ark_r1cs_std::alloc::AllocVar;
        use ark_relations::gr1cs::ConstraintSystem;

        let sw = {
            let pp = canonical_public_params().expect("canonical pp");
            let (rs, _d) = generate_fixture_with_digest(2).expect("fixture");
            extract_secondary_r1cs_witness(&rs, &pp).expect("extract secondary R1CS")
            // pp, rs dropped here; sw is owned (no JSON retained).
        };

        // Positive: the real secondary instance must be satisfied.
        let cs = ConstraintSystem::<ark_bn254::Fr>::new_ref();
        let mk = |v: ArkFq| NnFq::new_witness(cs.clone(), || Ok(v)).unwrap();
        let w: Vec<NnFq> = sw.w.iter().map(|&v| mk(v)).collect();
        let e: Vec<NnFq> = sw.e.iter().map(|&v| mk(v)).collect();
        let u = mk(sw.u);
        let x = [mk(sw.x[0]), mk(sw.x[1])];
        enforce_secondary_relaxed_r1cs_sat_nn(
            &w,
            &e,
            &u,
            &x,
            &sw.a_rows,
            &sw.b_rows,
            &sw.c_rows,
            sw.num_cons,
        )
        .expect("synthesize full secondary R1CS");
        assert!(
            cs.is_satisfied().expect("is_satisfied"),
            "real secondary RelaxedR1CS instance must be SATISFIED"
        );

        // Adversarial: perturb W[0] → some row must break.
        let cs2 = ConstraintSystem::<ark_bn254::Fr>::new_ref();
        let mk2 = |v: ArkFq| NnFq::new_witness(cs2.clone(), || Ok(v)).unwrap();
        let mut wbad = sw.w.clone();
        wbad[0] += ArkFq::from(1u64);
        let w2: Vec<NnFq> = wbad.iter().map(|&v| mk2(v)).collect();
        let e2: Vec<NnFq> = sw.e.iter().map(|&v| mk2(v)).collect();
        let u2 = mk2(sw.u);
        let x2 = [mk2(sw.x[0]), mk2(sw.x[1])];
        enforce_secondary_relaxed_r1cs_sat_nn(
            &w2,
            &e2,
            &u2,
            &x2,
            &sw.a_rows,
            &sw.b_rows,
            &sw.c_rows,
            sw.num_cons,
        )
        .expect("synthesize adv");
        assert!(
            !cs2.is_satisfied().expect("is_satisfied"),
            "perturbed W must make the secondary R1CS UNSATISFIABLE"
        );
    }

    /// **D.3 SIZE PREDICTION (tractable; the solution to "how big a
    /// machine")** — measures the non-native gadget's exact
    /// `num_constraints()` (memory-free, deterministic) on tiny
    /// synthetic satisfied instances, derives constraints-per-row +
    /// per-nonzero, then multiplies by the REAL secondary dims (D.2
    /// extraction on a real fixture) to PREDICT full-D.3 cost —
    /// without ever building full D.3. Runs on a 16 GB Mini.
    #[test]
    #[ignore = "D.3 size-prediction: tiny synthetic sweep + real-dims extract (tractable)"]
    fn secondary_r1cs_size_prediction() {
        use crate::recursive_snark_fixture::{
            canonical_public_params, generate_fixture_with_digest,
        };
        use crate::s4b_secondary_r1cs_gadget::{
            enforce_secondary_relaxed_r1cs_sat_nn, NnFq, SparseRow,
        };
        use ark_r1cs_std::alloc::AllocVar;
        use ark_relations::gr1cs::ConstraintSystem;

        // Synthetic SATISFIED instance: num_vars=num_cons=s, num_io=2,
        // `d` nonzeros/row at col 0. Row: A=B=C=[(0,1)], w[0]=1, u=1,
        // e=0 ⇒ 1·1 == 1·1 + 0. nnz/row = 3·dummy (we vary `d` by
        // repeating col-0 entries to scale nnz independently of rows).
        let measure = |s: usize, d: usize| -> usize {
            let cs = ConstraintSystem::<ark_bn254::Fr>::new_ref();
            let mk = |v: u64| NnFq::new_witness(cs.clone(), || Ok(ArkFq::from(v))).unwrap();
            let w: Vec<NnFq> = (0..s).map(|i| mk(if i == 0 { 1 } else { 0 })).collect();
            let e: Vec<NnFq> = (0..s).map(|_| mk(0)).collect();
            let u = mk(1);
            let x = [mk(0), mk(0)];
            // d entries all at col 0 with coeffs summing to the
            // satisfied value: A,B = [(0,1)]; C = d entries (0,1) but
            // that changes the value — instead keep value fixed:
            // A=[(0,1)], B=[(0,1)], C = [(0,1)] repeated→ Cz=d·w0.
            // Re-satisfy: w0·w0 == u·(d·w0) + e ⇒ pick w0 s.t.
            // 1 == d + e ⇒ e=1−d won't be 0 for d>1. Simpler: hold
            // d=3 fixed (A,B,C one entry each) and vary ONLY s; the
            // per-row cost (incl 3 non-native muls) is the slope.
            let _ = d;
            let row = || -> SparseRow { vec![(0usize, ArkFq::from(1u64))] };
            let a: Vec<SparseRow> = (0..s).map(|_| row()).collect();
            let b = a.clone();
            let c = a.clone();
            enforce_secondary_relaxed_r1cs_sat_nn(&w, &e, &u, &x, &a, &b, &c, s)
                .expect("synth gadget");
            assert!(cs.is_satisfied().expect("sat"), "synthetic must satisfy");
            cs.num_constraints()
        };

        let pts: Vec<(usize, usize)> = [8usize, 16, 32, 64]
            .iter()
            .map(|&s| (s, measure(s, 3)))
            .collect();
        eprintln!("SYNTHETIC SWEEP (s, num_constraints, d=3 nnz/row):");
        for (s, c) in &pts {
            eprintln!("  s={s:>3}  constraints={c}");
        }
        // Linear fit constraints ≈ k·s + b over the two extreme pts.
        let (s0, c0) = pts[0];
        let (s1, c1) = pts[pts.len() - 1];
        let k = (c1 as f64 - c0 as f64) / (s1 as f64 - s0 as f64);
        let b = c0 as f64 - k * s0 as f64;
        // k = constraints per (row + 3 non-native muls). per-nnz ≈ k/3
        // (each row's 3 nonzeros dominate; row/var overhead amortised).
        let per_nnz = k / 3.0;
        eprintln!("FIT: constraints ≈ {k:.1}·s + {b:.0}  ⇒  ~{per_nnz:.1}/nonzero");

        // REAL secondary dims (D.2 — proven, tractable ~30 s).
        let sw = {
            let pp = canonical_public_params().expect("pp");
            let (rs, _d) = generate_fixture_with_digest(2).expect("fixture");
            extract_secondary_r1cs_witness(&rs, &pp).expect("extract")
        };
        let nnz: usize = sw.a_rows.iter().map(|r| r.len()).sum::<usize>()
            + sw.b_rows.iter().map(|r| r.len()).sum::<usize>()
            + sw.c_rows.iter().map(|r| r.len()).sum::<usize>();
        eprintln!(
            "REAL secondary: num_cons={} num_vars={} total_nnz(A+B+C)={}",
            sw.num_cons, sw.num_vars, nnz
        );
        let predicted = per_nnz * nnz as f64 + k * sw.num_cons as f64;
        // Honest memory band: arkworks Groth16 setup+witness empirically
        // ~0.3–1.5 KB per constraint (PK + R1CS matrices + witness).
        let lo_gb = predicted * 300.0 / 1.073e9;
        let hi_gb = predicted * 1500.0 / 1.073e9;
        eprintln!(
            "PREDICTED full-D.3 ≈ {predicted:.3e} constraints  ⇒  ≈ {lo_gb:.0}–{hi_gb:.0} GB \
             (0.3–1.5 KB/constraint heuristic — ORDER-OF-MAGNITUDE, not precise)"
        );
        eprintln!(
            "CONCLUSION: if hi_gb ≫ available, the non-native path is the \
             wrong lever — curve-cycle redesign (B.0 Opt 3) is the real fix."
        );
        // No assertion on the prediction — it is a measurement, not a
        // pass/fail; the eprintln output IS the deliverable.
    }
}
