//! Phase 2 Section 2 sponge framing — port slice 1 of N.
//!
//! In-circuit port of neptune's optimized full-round permutation
//! step, mirroring `Poseidon::full_round` from
//! `nova-snark-0.68/src/frontend/gadgets/poseidon/poseidon_inner.rs:344-380`.
//!
//! See `SECTION_2_SPONGE_FRAMING.md` for the architectural
//! rationale + the 5-PR breakdown of which this is the first slice.
//!
//! # What's in this slice
//!
//! - [`neptune_full_round_native`] — pure off-circuit reference
//!   implementing `state[i] := state[i]^5 + post_ark[i]` for all
//!   `i`, then `state := mds · state`. Mirrors neptune's
//!   `full_round(false)` line-by-line: `quintic_s_box(l, None,
//!   post_key)` (S-Box, then add post-key), then
//!   `round_product_mds`.
//!
//! - [`enforce_neptune_full_round`] — the same recurrence encoded
//!   as arkworks R1CS constraints. Takes `&mut Vec<FpVar<F>>` and
//!   modifies it in place.
//!
//! # What's NOT in this slice
//!
//! - Partial rounds (next slice).
//! - Sparse-MDS handling (next-next slice).
//! - Full `permute()` orchestration (later).
//! - Wire-in to `section2_gadget` (last slice; canary flip happens then).
//!
//! # Bit-correctness contract
//!
//! [`enforce_neptune_full_round`] must produce witnesses that
//! equal [`neptune_full_round_native`] applied to the same inputs.
//! The companion tests pin this for randomised state vectors at
//! both width 3 (smallest non-trivial) and width 25 (the chain's
//! Poseidon parameter).

use ark_ff::PrimeField;
use ark_r1cs_std::alloc::AllocVar;
use ark_r1cs_std::eq::EqGadget;
use ark_r1cs_std::fields::fp::FpVar;
use ark_r1cs_std::fields::FieldVar;
use ark_relations::r1cs::{ConstraintSystemRef, SynthesisError};

/// Off-circuit reference for one neptune **full** round:
///
/// 1. For every state cell `i`, compute `state[i] := state[i]^5
///    + post_ark[i]` — the SBOX-trick form (post-add fused).
/// 2. Multiply by the MDS matrix: `state[i] := sum_j(mds[i][j]
///    * state[j])`.
///
/// `state.len()` must equal `post_ark.len()` must equal
/// `mds.len()` must equal `mds[0].len()` (square matrix matching
/// the state width).
pub fn neptune_full_round_native<F: PrimeField>(
    state: &mut [F],
    post_ark: &[F],
    mds: &[Vec<F>],
) {
    let width = state.len();
    assert_eq!(post_ark.len(), width, "post_ark length must match state width");
    assert_eq!(mds.len(), width, "mds row count must match state width");
    for row in mds.iter() {
        assert_eq!(row.len(), width, "mds row width must match state width");
    }

    // Step 1: S-Box + post-add fused (the SBOX-trick).
    for i in 0..width {
        let s = state[i];
        let s2 = s * s;
        let s4 = s2 * s2;
        state[i] = s * s4 + post_ark[i]; // s^5 + post
    }

    // Step 2: MDS multiplication (out := mds · state).
    let mut out = vec![F::zero(); width];
    for (i, row) in mds.iter().enumerate() {
        let mut acc = F::zero();
        for (j, m) in row.iter().enumerate() {
            acc += *m * state[j];
        }
        out[i] = acc;
    }
    state.copy_from_slice(&out);
}

/// In-circuit equivalent of [`neptune_full_round_native`]. Mutates
/// `state` in place with new `FpVar<F>` cells holding the
/// post-round witnesses, and emits the R1CS constraints that bind
/// the new cells to the SBOX-trick + MDS recurrence on the old
/// cells.
///
/// Cost: 3 multiplications per S-Box (`s^2`, `s^4`, `s^5 + post`)
/// × `width` cells + `width × width` multiplications for the MDS
/// matmul. For width = 25 (chain Poseidon): 75 + 625 = ~700
/// constraints per full round.
pub fn enforce_neptune_full_round<F: PrimeField>(
    _cs: ConstraintSystemRef<F>,
    state: &mut Vec<FpVar<F>>,
    post_ark: &[F],
    mds: &[Vec<F>],
) -> Result<(), SynthesisError> {
    let width = state.len();
    assert_eq!(post_ark.len(), width, "post_ark length must match state width");
    assert_eq!(mds.len(), width, "mds row count must match state width");
    for row in mds.iter() {
        assert_eq!(row.len(), width, "mds row width must match state width");
    }

    // Step 1: in-circuit SBOX-trick — for each cell:
    //   s2  := s * s        (1 constraint)
    //   s4  := s2 * s2      (1 constraint)
    //   s5  := s * s4       (1 constraint)
    //   out := s5 + post    (no constraint — constant add)
    for i in 0..width {
        let s = state[i].clone();
        let s2 = &s * &s;
        let s4 = &s2 * &s2;
        let s5 = &s * &s4;
        // post_ark[i] is a constant — multiplying it through the
        // constraint system as a constant FpVar avoids paying for
        // a witness allocation.
        let post = FpVar::<F>::constant(post_ark[i]);
        state[i] = s5 + post;
    }

    // Step 2: MDS multiplication. Each output cell is a dot
    // product of a constant matrix row with the current state.
    let mut out: Vec<FpVar<F>> = Vec::with_capacity(width);
    for row in mds.iter() {
        let mut acc = FpVar::<F>::constant(F::zero());
        for (j, m) in row.iter().enumerate() {
            let m_const = FpVar::<F>::constant(*m);
            acc += &m_const * &state[j];
        }
        out.push(acc);
    }
    state.clear();
    state.extend(out);

    Ok(())
}

/// Off-circuit reference for one neptune **partial** round:
///
/// 1. Apply the SBOX-trick only to `state[0]`: `state[0] :=
///    state[0]^5 + post_ark`. All other state cells are
///    untouched at this step.
/// 2. Multiply by the MDS matrix: `state[i] := sum_j(mds[i][j]
///    * state[j])` for every row `i`.
///
/// Mirrors neptune's `partial_round`
/// (`nova-snark-0.68/src/frontend/gadgets/poseidon/poseidon_inner.rs:382-392`).
///
/// `post_ark` is a single scalar (vs the full round's vector) —
/// neptune's compressed round constants emit ONE entry per
/// partial round, not `width` entries.
pub fn neptune_partial_round_native<F: PrimeField>(
    state: &mut [F],
    post_ark: F,
    mds: &[Vec<F>],
) {
    let width = state.len();
    assert_eq!(mds.len(), width, "mds row count must match state width");
    for row in mds.iter() {
        assert_eq!(row.len(), width, "mds row width must match state width");
    }

    // Step 1: S-Box + post-add on state[0] ONLY.
    let s = state[0];
    let s2 = s * s;
    let s4 = s2 * s2;
    state[0] = s * s4 + post_ark;

    // Step 2: MDS multiplication (out := mds · state).
    let mut out = vec![F::zero(); width];
    for (i, row) in mds.iter().enumerate() {
        let mut acc = F::zero();
        for (j, m) in row.iter().enumerate() {
            acc += *m * state[j];
        }
        out[i] = acc;
    }
    state.copy_from_slice(&out);
}

/// In-circuit equivalent of [`neptune_partial_round_native`].
///
/// Cost at width 25: 3 mults for the single S-Box + 25×25 = 625
/// mults for the MDS matmul = ~628 constraints per partial round.
/// Cheaper than a full round (which has 75 S-Box mults).
///
/// Sparse-MDS optimization (which would replace the 625 with O(1)
/// constraints) is slice 3 of the port plan, not this slice.
pub fn enforce_neptune_partial_round<F: PrimeField>(
    _cs: ConstraintSystemRef<F>,
    state: &mut Vec<FpVar<F>>,
    post_ark: F,
    mds: &[Vec<F>],
) -> Result<(), SynthesisError> {
    let width = state.len();
    assert_eq!(mds.len(), width, "mds row count must match state width");
    for row in mds.iter() {
        assert_eq!(row.len(), width, "mds row width must match state width");
    }

    // Step 1: in-circuit SBOX-trick on state[0] only.
    let s = state[0].clone();
    let s2 = &s * &s;
    let s4 = &s2 * &s2;
    let s5 = &s * &s4;
    let post = FpVar::<F>::constant(post_ark);
    state[0] = s5 + post;

    // Step 2: MDS multiplication — same shape as full-round Step 2.
    let mut out: Vec<FpVar<F>> = Vec::with_capacity(width);
    for row in mds.iter() {
        let mut acc = FpVar::<F>::constant(F::zero());
        for (j, m) in row.iter().enumerate() {
            let m_const = FpVar::<F>::constant(*m);
            acc += &m_const * &state[j];
        }
        out.push(acc);
    }
    state.clear();
    state.extend(out);

    Ok(())
}

/// Neptune-style sparse matrix used in the fast partial-round MDS.
///
/// Neptune factors the partial-round MDS into `M = M' · M_sparse`
/// where `M_sparse` has only `2·width - 1` non-trivial entries:
///
/// ```text
///   [ w_hat[0]    v_rest[0]   v_rest[1]   ...   v_rest[w-2] ]
///   [ w_hat[1]    1           0           ...   0           ]
///   [ w_hat[2]    0           1           ...   0           ]
///   [ ...                                       ...         ]
///   [ w_hat[w-1]  0           0           ...   1           ]
/// ```
///
/// Layout mirrors neptune's `SparseMatrix` struct
/// (`nova-snark-0.68/src/frontend/gadgets/poseidon/poseidon_inner.rs`).
/// Multiplication `out = M_sparse · state` costs `2·width - 1`
/// scalar mults instead of `width²` for a generic MDS.
#[derive(Clone, Debug)]
pub struct NeptuneSparseMatrix<F: PrimeField> {
    /// First column of the sparse matrix (length = width).
    pub w_hat: Vec<F>,
    /// First row beyond column 0 (length = width - 1).
    pub v_rest: Vec<F>,
}

impl<F: PrimeField> NeptuneSparseMatrix<F> {
    /// Construct a sparse matrix from its `w_hat` column + `v_rest` row.
    ///
    /// Panics if `v_rest.len() != w_hat.len() - 1`.
    pub fn new(w_hat: Vec<F>, v_rest: Vec<F>) -> Self {
        assert_eq!(
            v_rest.len() + 1,
            w_hat.len(),
            "v_rest must have length width-1 where width = w_hat.len()"
        );
        Self { w_hat, v_rest }
    }

    /// Width of the sparse matrix (= state width).
    pub fn width(&self) -> usize {
        self.w_hat.len()
    }
}

/// Off-circuit reference for one neptune **partial** round with the
/// **sparse-MDS fast path** (mirrors neptune's
/// `product_mds_with_sparse_matrix`):
///
/// 1. SBOX-trick on `state[0]` only:
///    `state[0] := state[0]^5 + post_ark`
/// 2. Sparse-MDS multiplication:
///    - `out[0] := sum_i(w_hat[i] * state[i])`
///    - `out[j>0] := state[j] + v_rest[j-1] * state[0]`
///
/// Cost: `2·width - 1` mults vs `width²` for a plain-MDS partial
/// round. At width 25: 49 vs 625 mults (~12.7× speedup).
pub fn neptune_partial_round_sparse_native<F: PrimeField>(
    state: &mut [F],
    post_ark: F,
    sparse: &NeptuneSparseMatrix<F>,
) {
    let width = state.len();
    assert_eq!(
        sparse.width(),
        width,
        "sparse matrix width must match state width"
    );

    // Step 1: SBOX + post-add on state[0] only.
    let s = state[0];
    let s2 = s * s;
    let s4 = s2 * s2;
    state[0] = s * s4 + post_ark;

    // Step 2: sparse-MDS multiplication.
    let mut out = vec![F::zero(); width];

    // out[0] = sum_i(w_hat[i] * state[i]) — dense first column.
    for i in 0..width {
        out[0] += sparse.w_hat[i] * state[i];
    }

    // out[j>0] = state[j] + v_rest[j-1] * state[0] — identity diagonal
    // plus dense first row's contribution from state[0].
    for j in 1..width {
        out[j] = state[j] + sparse.v_rest[j - 1] * state[0];
    }

    state.copy_from_slice(&out);
}

/// In-circuit equivalent of [`neptune_partial_round_sparse_native`].
///
/// **R1CS constraint cost: 3 (same as the plain-MDS partial round).**
/// arkworks folds every `FpVar::constant * FpVar` multiplication
/// into a linear combination at zero constraint cost — only the
/// SBOX (`s²`, `s⁴`, `s⁵`) actually consumes constraints. So both
/// sparse and plain paths cost exactly the 3 SBOX mults when the
/// MDS entries are config-time constants (our case). See the test
/// `sparse_and_plain_partial_round_have_same_constraint_cost_under_constant_mds`.
///
/// **Why slice 3 exists, then.** The value isn't constraint savings
/// in R1CS — it's STRUCTURAL byte-parity match with neptune's
/// permutation shape. Neptune switches between plain MDS, pre-sparse
/// matrix, and sparse matrices at different rounds; slice 4
/// (`permute()` orchestration) needs to call into the same shape
/// neptune uses to produce bit-identical output.
pub fn enforce_neptune_partial_round_sparse<F: PrimeField>(
    _cs: ConstraintSystemRef<F>,
    state: &mut Vec<FpVar<F>>,
    post_ark: F,
    sparse: &NeptuneSparseMatrix<F>,
) -> Result<(), SynthesisError> {
    let width = state.len();
    assert_eq!(
        sparse.width(),
        width,
        "sparse matrix width must match state width"
    );

    // Step 1: SBOX-trick on state[0] only — identical to slice 2's
    // plain partial-round Step 1.
    let s = state[0].clone();
    let s2 = &s * &s;
    let s4 = &s2 * &s2;
    let s5 = &s * &s4;
    let post = FpVar::<F>::constant(post_ark);
    state[0] = s5 + post;

    // Step 2: sparse-MDS multiplication.
    let mut out: Vec<FpVar<F>> = Vec::with_capacity(width);

    // out[0] = sum_i(w_hat[i] * state[i]) — width mults.
    let mut acc = FpVar::<F>::constant(F::zero());
    for i in 0..width {
        let m = FpVar::<F>::constant(sparse.w_hat[i]);
        acc += &m * &state[i];
    }
    out.push(acc);

    // out[j>0] = state[j] + v_rest[j-1] * state[0] — (width-1) mults.
    for j in 1..width {
        let m = FpVar::<F>::constant(sparse.v_rest[j - 1]);
        let cell = &state[j] + &m * &state[0];
        out.push(cell);
    }

    state.clear();
    state.extend(out);

    Ok(())
}

/// Convenience: assert two state vectors are bit-equal in the
/// constraint system. Used by tests to pin gadget output against
/// the native reference.
pub fn enforce_state_eq<F: PrimeField>(
    a: &[FpVar<F>],
    b: &[FpVar<F>],
) -> Result<(), SynthesisError> {
    assert_eq!(a.len(), b.len(), "state widths must match for equality check");
    for (x, y) in a.iter().zip(b.iter()) {
        x.enforce_equal(y)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_bn254::Fr as Bn254Fr;
    use ark_r1cs_std::R1CSVar;
    use ark_relations::r1cs::ConstraintSystem;

    /// Hand-computed sanity: width-3 state, identity MDS, zero
    /// post-ARK → after one full round, state[i] = state[i]^5.
    /// Pin against literal small values so a regression in the
    /// SBOX recurrence is loud.
    #[test]
    fn native_full_round_identity_mds_zero_ark_is_pure_pow5() {
        let mut state = vec![Bn254Fr::from(0u64), Bn254Fr::from(1u64), Bn254Fr::from(2u64)];
        let ark = vec![Bn254Fr::from(0u64); 3];
        let identity_mds = vec![
            vec![Bn254Fr::from(1u64), Bn254Fr::from(0u64), Bn254Fr::from(0u64)],
            vec![Bn254Fr::from(0u64), Bn254Fr::from(1u64), Bn254Fr::from(0u64)],
            vec![Bn254Fr::from(0u64), Bn254Fr::from(0u64), Bn254Fr::from(1u64)],
        ];

        neptune_full_round_native(&mut state, &ark, &identity_mds);
        assert_eq!(state[0], Bn254Fr::from(0u64), "0^5 == 0");
        assert_eq!(state[1], Bn254Fr::from(1u64), "1^5 == 1");
        assert_eq!(state[2], Bn254Fr::from(32u64), "2^5 == 32");
    }

    /// Width-3 with non-trivial post-ARK and a 2×identity MDS:
    ///   intermediate after SBOX+post: [0^5+7, 1^5+11, 2^5+13]
    ///                                = [7, 12, 45]
    ///   after MDS × 2: [14, 24, 90].
    #[test]
    fn native_full_round_scaled_mds_with_nonzero_ark() {
        let mut state = vec![Bn254Fr::from(0u64), Bn254Fr::from(1u64), Bn254Fr::from(2u64)];
        let ark = vec![Bn254Fr::from(7u64), Bn254Fr::from(11u64), Bn254Fr::from(13u64)];
        let scaled_mds = vec![
            vec![Bn254Fr::from(2u64), Bn254Fr::from(0u64), Bn254Fr::from(0u64)],
            vec![Bn254Fr::from(0u64), Bn254Fr::from(2u64), Bn254Fr::from(0u64)],
            vec![Bn254Fr::from(0u64), Bn254Fr::from(0u64), Bn254Fr::from(2u64)],
        ];

        neptune_full_round_native(&mut state, &ark, &scaled_mds);
        assert_eq!(state[0], Bn254Fr::from(14u64), "(0^5+7) * 2");
        assert_eq!(state[1], Bn254Fr::from(24u64), "(1^5+11) * 2");
        assert_eq!(state[2], Bn254Fr::from(90u64), "(2^5+13) * 2");
    }

    /// **The bit-correctness pin.** Width-3, hand-picked
    /// non-trivial state + ARK + MDS. The in-circuit gadget
    /// must produce witnesses bit-equal to the native reference.
    #[test]
    fn gadget_matches_native_on_width_3() {
        let init_state = vec![Bn254Fr::from(3u64), Bn254Fr::from(5u64), Bn254Fr::from(7u64)];
        let post_ark = vec![Bn254Fr::from(11u64), Bn254Fr::from(13u64), Bn254Fr::from(17u64)];
        let mds = vec![
            vec![Bn254Fr::from(2u64), Bn254Fr::from(3u64), Bn254Fr::from(5u64)],
            vec![Bn254Fr::from(7u64), Bn254Fr::from(11u64), Bn254Fr::from(13u64)],
            vec![Bn254Fr::from(17u64), Bn254Fr::from(19u64), Bn254Fr::from(23u64)],
        ];

        // Native reference.
        let mut native = init_state.clone();
        neptune_full_round_native(&mut native, &post_ark, &mds);

        // In-circuit.
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        let mut state_vars: Vec<FpVar<Bn254Fr>> = init_state
            .iter()
            .map(|s| FpVar::new_witness(cs.clone(), || Ok(*s)).expect("alloc"))
            .collect();
        enforce_neptune_full_round(cs.clone(), &mut state_vars, &post_ark, &mds)
            .expect("synthesize");

        // Read witnesses + assert bit-equal to native.
        for (i, (var, expected)) in state_vars.iter().zip(native.iter()).enumerate() {
            let v = var.value().expect("witness value");
            assert_eq!(
                v, *expected,
                "gadget state[{i}] {v:?} != native {expected:?}"
            );
        }

        // CS must be satisfied.
        assert!(cs.is_satisfied().expect("is_satisfied"));
    }

    /// Same pin at width 25 (the chain's Poseidon parameter).
    /// Uses a deterministic PRG to fill state + ARK + MDS so the
    /// test is reproducible without locking pinned vectors that
    /// would force re-pinning at any unrelated refactor.
    #[test]
    fn gadget_matches_native_on_width_25() {
        // Deterministic Fr stream — use simple linear sequence
        // 1, 2, 3, ... mod field. Good enough for a bit-parity
        // test (no need for cryptographic randomness here).
        let next_fr = |k: u64| Bn254Fr::from(k.wrapping_mul(0x9E37_79B9_7F4A_7C15));

        let width = 25;
        let init_state: Vec<Bn254Fr> = (0..width).map(|i| next_fr(i as u64 + 1)).collect();
        let post_ark: Vec<Bn254Fr> = (0..width).map(|i| next_fr(i as u64 + 100)).collect();
        let mds: Vec<Vec<Bn254Fr>> = (0..width)
            .map(|i| {
                (0..width)
                    .map(|j| next_fr(1000 + (i * width + j) as u64))
                    .collect()
            })
            .collect();

        // Native.
        let mut native = init_state.clone();
        neptune_full_round_native(&mut native, &post_ark, &mds);

        // In-circuit.
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        let mut state_vars: Vec<FpVar<Bn254Fr>> = init_state
            .iter()
            .map(|s| FpVar::new_witness(cs.clone(), || Ok(*s)).expect("alloc"))
            .collect();
        enforce_neptune_full_round(cs.clone(), &mut state_vars, &post_ark, &mds)
            .expect("synthesize");

        for (i, (var, expected)) in state_vars.iter().zip(native.iter()).enumerate() {
            let v = var.value().expect("witness value");
            assert_eq!(
                v, *expected,
                "gadget state[{i}] != native at width 25"
            );
        }
        assert!(cs.is_satisfied().expect("is_satisfied"));
    }

    /// Partial round, hand-computed: width 3, state [3, 5, 7],
    /// post_ark = 11, identity MDS.
    ///   After SBOX on state[0]: state = [3^5+11, 5, 7] = [254, 5, 7].
    ///   After identity MDS: unchanged.
    #[test]
    fn native_partial_round_identity_mds_only_state_0_changes() {
        let mut state = vec![Bn254Fr::from(3u64), Bn254Fr::from(5u64), Bn254Fr::from(7u64)];
        let identity_mds = vec![
            vec![Bn254Fr::from(1u64), Bn254Fr::from(0u64), Bn254Fr::from(0u64)],
            vec![Bn254Fr::from(0u64), Bn254Fr::from(1u64), Bn254Fr::from(0u64)],
            vec![Bn254Fr::from(0u64), Bn254Fr::from(0u64), Bn254Fr::from(1u64)],
        ];

        neptune_partial_round_native(&mut state, Bn254Fr::from(11u64), &identity_mds);
        assert_eq!(state[0], Bn254Fr::from(254u64), "3^5 + 11 = 243 + 11 = 254");
        assert_eq!(state[1], Bn254Fr::from(5u64), "state[1] untouched by SBOX");
        assert_eq!(state[2], Bn254Fr::from(7u64), "state[2] untouched by SBOX");
    }

    /// Partial round with a non-trivial MDS — confirms the MDS
    /// step mixes the (SBOX-applied) state[0] into all output
    /// cells.
    ///   pre-MDS:  state = [3^5+11, 5, 7] = [254, 5, 7]
    ///   MDS row 0: [2, 0, 0] → 2 × 254 = 508
    ///   MDS row 1: [0, 3, 0] → 3 × 5 = 15
    ///   MDS row 2: [1, 1, 1] → 254 + 5 + 7 = 266
    #[test]
    fn native_partial_round_mds_mixes_state_0_into_other_cells() {
        let mut state = vec![Bn254Fr::from(3u64), Bn254Fr::from(5u64), Bn254Fr::from(7u64)];
        let mds = vec![
            vec![Bn254Fr::from(2u64), Bn254Fr::from(0u64), Bn254Fr::from(0u64)],
            vec![Bn254Fr::from(0u64), Bn254Fr::from(3u64), Bn254Fr::from(0u64)],
            vec![Bn254Fr::from(1u64), Bn254Fr::from(1u64), Bn254Fr::from(1u64)],
        ];

        neptune_partial_round_native(&mut state, Bn254Fr::from(11u64), &mds);
        assert_eq!(state[0], Bn254Fr::from(508u64), "2 * (3^5+11) = 508");
        assert_eq!(state[1], Bn254Fr::from(15u64), "3 * 5 = 15");
        assert_eq!(
            state[2],
            Bn254Fr::from(266u64),
            "(3^5+11) + 5 + 7 = 266 (sum-row mixes state[0] in)"
        );
    }

    /// **Bit-correctness pin** for partial round at width 3.
    #[test]
    fn partial_gadget_matches_native_on_width_3() {
        let init_state = vec![Bn254Fr::from(3u64), Bn254Fr::from(5u64), Bn254Fr::from(7u64)];
        let post_ark = Bn254Fr::from(11u64);
        let mds = vec![
            vec![Bn254Fr::from(2u64), Bn254Fr::from(3u64), Bn254Fr::from(5u64)],
            vec![Bn254Fr::from(7u64), Bn254Fr::from(11u64), Bn254Fr::from(13u64)],
            vec![Bn254Fr::from(17u64), Bn254Fr::from(19u64), Bn254Fr::from(23u64)],
        ];

        let mut native = init_state.clone();
        neptune_partial_round_native(&mut native, post_ark, &mds);

        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        let mut state_vars: Vec<FpVar<Bn254Fr>> = init_state
            .iter()
            .map(|s| FpVar::new_witness(cs.clone(), || Ok(*s)).expect("alloc"))
            .collect();
        enforce_neptune_partial_round(cs.clone(), &mut state_vars, post_ark, &mds)
            .expect("synthesize");

        for (i, (var, expected)) in state_vars.iter().zip(native.iter()).enumerate() {
            let v = var.value().expect("witness value");
            assert_eq!(
                v, *expected,
                "partial gadget state[{i}] {v:?} != native {expected:?}"
            );
        }
        assert!(cs.is_satisfied().expect("is_satisfied"));
    }

    /// Bit-correctness pin for partial round at chain width 25.
    #[test]
    fn partial_gadget_matches_native_on_width_25() {
        let next_fr = |k: u64| Bn254Fr::from(k.wrapping_mul(0x9E37_79B9_7F4A_7C15));

        let width = 25;
        let init_state: Vec<Bn254Fr> = (0..width).map(|i| next_fr(i as u64 + 1)).collect();
        let post_ark = next_fr(999);
        let mds: Vec<Vec<Bn254Fr>> = (0..width)
            .map(|i| {
                (0..width)
                    .map(|j| next_fr(2000 + (i * width + j) as u64))
                    .collect()
            })
            .collect();

        let mut native = init_state.clone();
        neptune_partial_round_native(&mut native, post_ark, &mds);

        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        let mut state_vars: Vec<FpVar<Bn254Fr>> = init_state
            .iter()
            .map(|s| FpVar::new_witness(cs.clone(), || Ok(*s)).expect("alloc"))
            .collect();
        enforce_neptune_partial_round(cs.clone(), &mut state_vars, post_ark, &mds)
            .expect("synthesize");

        for (i, (var, expected)) in state_vars.iter().zip(native.iter()).enumerate() {
            let v = var.value().expect("witness value");
            assert_eq!(
                v, *expected,
                "partial gadget state[{i}] != native at width 25"
            );
        }
        assert!(cs.is_satisfied().expect("is_satisfied"));
    }

    // ── Slice 3: sparse-MDS partial round ────────────────────────

    /// Hand-computed sparse-MDS native pin at width 3.
    /// State [3, 5, 7], post_ark = 11.
    ///   After SBOX on state[0]: [3^5+11, 5, 7] = [254, 5, 7]
    /// Sparse matrix with w_hat = [2, 3, 5], v_rest = [7, 11]:
    ///   out[0] = 2·254 + 3·5 + 5·7 = 508 + 15 + 35 = 558
    ///   out[1] = 5 + 7·254 = 5 + 1778 = 1783
    ///   out[2] = 7 + 11·254 = 7 + 2794 = 2801
    #[test]
    fn native_partial_round_sparse_hand_computed() {
        let mut state = vec![Bn254Fr::from(3u64), Bn254Fr::from(5u64), Bn254Fr::from(7u64)];
        let sparse = NeptuneSparseMatrix::new(
            vec![Bn254Fr::from(2u64), Bn254Fr::from(3u64), Bn254Fr::from(5u64)],
            vec![Bn254Fr::from(7u64), Bn254Fr::from(11u64)],
        );
        neptune_partial_round_sparse_native(&mut state, Bn254Fr::from(11u64), &sparse);
        assert_eq!(state[0], Bn254Fr::from(558u64), "2·254 + 3·5 + 5·7 = 558");
        assert_eq!(state[1], Bn254Fr::from(1783u64), "5 + 7·254 = 1783");
        assert_eq!(state[2], Bn254Fr::from(2801u64), "7 + 11·254 = 2801");
    }

    /// **Bit-correctness pin** for sparse-MDS partial round at width 3.
    #[test]
    fn sparse_gadget_matches_native_on_width_3() {
        let init_state = vec![Bn254Fr::from(3u64), Bn254Fr::from(5u64), Bn254Fr::from(7u64)];
        let post_ark = Bn254Fr::from(11u64);
        let sparse = NeptuneSparseMatrix::new(
            vec![Bn254Fr::from(2u64), Bn254Fr::from(3u64), Bn254Fr::from(5u64)],
            vec![Bn254Fr::from(7u64), Bn254Fr::from(11u64)],
        );

        let mut native = init_state.clone();
        neptune_partial_round_sparse_native(&mut native, post_ark, &sparse);

        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        let mut state_vars: Vec<FpVar<Bn254Fr>> = init_state
            .iter()
            .map(|s| FpVar::new_witness(cs.clone(), || Ok(*s)).expect("alloc"))
            .collect();
        enforce_neptune_partial_round_sparse(cs.clone(), &mut state_vars, post_ark, &sparse)
            .expect("synthesize");

        for (i, (var, expected)) in state_vars.iter().zip(native.iter()).enumerate() {
            let v = var.value().expect("witness value");
            assert_eq!(
                v, *expected,
                "sparse gadget state[{i}] {v:?} != native {expected:?}"
            );
        }
        assert!(cs.is_satisfied().expect("is_satisfied"));
    }

    /// Bit-correctness pin at chain Poseidon width 25 with PRG-filled
    /// sparse matrix.
    #[test]
    fn sparse_gadget_matches_native_on_width_25() {
        let next_fr = |k: u64| Bn254Fr::from(k.wrapping_mul(0x9E37_79B9_7F4A_7C15));

        let width = 25;
        let init_state: Vec<Bn254Fr> = (0..width).map(|i| next_fr(i as u64 + 1)).collect();
        let post_ark = next_fr(7777);
        let sparse = NeptuneSparseMatrix::new(
            (0..width).map(|i| next_fr(3000 + i as u64)).collect(),
            (0..width - 1).map(|i| next_fr(4000 + i as u64)).collect(),
        );

        let mut native = init_state.clone();
        neptune_partial_round_sparse_native(&mut native, post_ark, &sparse);

        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        let mut state_vars: Vec<FpVar<Bn254Fr>> = init_state
            .iter()
            .map(|s| FpVar::new_witness(cs.clone(), || Ok(*s)).expect("alloc"))
            .collect();
        enforce_neptune_partial_round_sparse(cs.clone(), &mut state_vars, post_ark, &sparse)
            .expect("synthesize");

        for (i, (var, expected)) in state_vars.iter().zip(native.iter()).enumerate() {
            let v = var.value().expect("witness value");
            assert_eq!(
                v, *expected,
                "sparse gadget state[{i}] != native at width 25"
            );
        }
        assert!(cs.is_satisfied().expect("is_satisfied"));
    }

    /// **Constraint-count finding (post-empirical).** With a CONSTANT
    /// MDS matrix, arkworks folds every `FpVar::constant * FpVar`
    /// multiplication into a linear combination at ZERO constraint
    /// cost — only the SBOX (`s²`, `s⁴`, `s⁵`) actually consumes
    /// constraints.
    ///
    /// So sparse-vs-plain at width 25 both produce 3 constraints
    /// (the 3 SBOX mults). The slice-3 sparse path's value is NOT
    /// constraint-count savings — it's STRUCTURAL byte-parity match
    /// with neptune's permutation shape (needed for slice 4
    /// `permute()` to call into the right matrix per round).
    ///
    /// This pin documents the empirical equality so future readers
    /// don't expect a constraint-count savings that doesn't exist
    /// in this R1CS setting. (It WOULD show up if the MDS was a
    /// witness, e.g. dynamic MDS, but ours is fixed at config time.)
    #[test]
    fn sparse_and_plain_partial_round_have_same_constraint_cost_under_constant_mds() {
        let next_fr = |k: u64| Bn254Fr::from(k.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let width = 25;
        let init_state: Vec<Bn254Fr> = (0..width).map(|i| next_fr(i as u64 + 1)).collect();
        let post_ark = next_fr(99);

        // Sparse path.
        let sparse = NeptuneSparseMatrix::new(
            (0..width).map(|i| next_fr(3000 + i as u64)).collect(),
            (0..width - 1).map(|i| next_fr(4000 + i as u64)).collect(),
        );
        let cs_sparse = ConstraintSystem::<Bn254Fr>::new_ref();
        let mut state_sparse: Vec<FpVar<Bn254Fr>> = init_state
            .iter()
            .map(|s| FpVar::new_witness(cs_sparse.clone(), || Ok(*s)).expect("alloc"))
            .collect();
        enforce_neptune_partial_round_sparse(
            cs_sparse.clone(),
            &mut state_sparse,
            post_ark,
            &sparse,
        )
        .expect("sparse synth");
        let sparse_constraints = cs_sparse.num_constraints();

        // Plain path (slice 2).
        let plain_mds: Vec<Vec<Bn254Fr>> = (0..width)
            .map(|i| {
                (0..width)
                    .map(|j| next_fr(5000 + (i * width + j) as u64))
                    .collect()
            })
            .collect();
        let cs_plain = ConstraintSystem::<Bn254Fr>::new_ref();
        let mut state_plain: Vec<FpVar<Bn254Fr>> = init_state
            .iter()
            .map(|s| FpVar::new_witness(cs_plain.clone(), || Ok(*s)).expect("alloc"))
            .collect();
        enforce_neptune_partial_round(cs_plain.clone(), &mut state_plain, post_ark, &plain_mds)
            .expect("plain synth");
        let plain_constraints = cs_plain.num_constraints();

        eprintln!("sparse: {sparse_constraints} constraints");
        eprintln!("plain:  {plain_constraints} constraints");

        // Empirical observation: both are exactly 3 (the SBOX mults).
        // arkworks folds constant×FpVar into linear combinations
        // at no constraint cost. Pin the equality.
        assert_eq!(
            sparse_constraints, plain_constraints,
            "with constant MDS, sparse and plain produce identical constraint count"
        );
        assert_eq!(
            sparse_constraints, 3,
            "both paths cost exactly the 3 SBOX mults (s², s⁴, s⁵) when MDS is constant"
        );
    }

    /// `enforce_state_eq` accepts identical state vectors.
    #[test]
    fn enforce_state_eq_accepts_identical_vectors() {
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        let a: Vec<FpVar<Bn254Fr>> = (0..5)
            .map(|i| FpVar::new_witness(cs.clone(), || Ok(Bn254Fr::from(i as u64))).expect("alloc"))
            .collect();
        let b: Vec<FpVar<Bn254Fr>> = (0..5)
            .map(|i| FpVar::new_witness(cs.clone(), || Ok(Bn254Fr::from(i as u64))).expect("alloc"))
            .collect();
        enforce_state_eq(&a, &b).expect("enforce_eq");
        assert!(cs.is_satisfied().expect("is_satisfied"));
    }
}
