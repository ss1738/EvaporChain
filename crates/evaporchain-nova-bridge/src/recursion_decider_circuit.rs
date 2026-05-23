//! B-1/B-2 EVM option (2): the BN254-Fr **final-layer recursion
//! decider circuit**.
//!
//! # Why this circuit exists
//!
//! `CompressedSNARK::verify` (nova-snark 0.68, `nova/mod.rs`
//! L909-1025) is entirely constant-size EXCEPT one term: the
//! **secondary** Spartan SNARK's IPA `ck_hat = Σ sᵢ·ckᵢ` size-`n`
//! MSM over Grumpkin (`ipa_pc.rs` L351). Doing that MSM **non-
//! natively** (Grumpkin scalar = BN254-Fq foreign field) is the
//! S4b/D.3 ~2.03×10⁸-constraint dead-end.
//!
//! Grumpkin's *base* field **is BN254-Fr** — the circuit-native
//! field here. So this circuit recomputes that one super-constant
//! term with **native** point arithmetic via
//! [`crate::s4_msm_gadget::pedersen_msm_grumpkin`]
//! (measured `predict_native_grumpkin_msm_size_for_recursion_circuit`
//! → ~2.67×10⁷ for n=10_554, ≪ D.3's 2.03×10⁸, falsifier did not
//! fire). The succinct (ppsnark) verifier of *this* circuit is then
//! Groth16-wrapped for the EVM (`groth16_wrapper` / `eip197`).
//!
//! # What this milestone ships (incremental — write→box→fix)
//!
//!   - **Section A — secondary `ck_hat` MSM binding [LIVE].** The
//!     load-bearing, dominant (~26.7M-constraint) term. Recompute
//!     `Σ sᵢ·ckᵢ + r·h` in-circuit and `enforce_equal` it to the
//!     claimed commitment. This section is REAL and box-verified by
//!     the tests below (positive: correct commitment ⇒ CS satisfied;
//!     negative: wrong commitment ⇒ CS unsatisfied — proves the
//!     binding actually constrains, not a vacuous gate).
//!
//!   - **Section B — Neptune hash anchors [DEFERRED stub].** The
//!     `hash_primary/secondary` Poseidon checks. Constant-size;
//!     gadget exists (`neptune_permutation_gadget`). Wired in a
//!     later increment.
//!
//!   - **Section C — NIFS folds + derandomize [DEFERRED stub].**
//!     Three constant-size `nifs_*.verify` folds and the commitment
//!     derandomization. Constant-size by construction.
//!
//!   - **Section D — primary HyperKZG pairing [DEFERRED stub].**
//!     One bounded-constant BN254 pairing (`snark_primary.verify`).
//!
//! Sections B-D are constant-size by the source analysis in
//! `MAINNET_REMAINING_WORK_FLOW.md` (source read #3); only Section A
//! is super-constant, so Section A is built and proven first. The
//! deferred sections are documented stubs, NOT silent omissions —
//! the struct carries an explicit `sections_bcd_wired: bool` so a
//! caller cannot mistake a Section-A-only instance for a complete
//! decider.

use ark_bn254::{Fq as Bn254Fq, Fr as Bn254Fr};
use ark_ec::short_weierstrass::{Affine, Projective};
use ark_r1cs_std::alloc::AllocVar;
use ark_r1cs_std::eq::EqGadget;
use ark_r1cs_std::fields::emulated_fp::EmulatedFpVar;
use ark_relations::gr1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};

use crate::grumpkin_config::GrumpkinConfig;
use crate::s4_msm_gadget::{pedersen_msm_grumpkin, GrumpkinVar};

/// Section B public-input bundle — the
/// `CompressedSNARK::verify` output-hash binding inputs
/// (per `SECTION_B_SCOPING.md`). Owned, plain data.
///
/// All fields are Bn254 Fr (the circuit's native field). The
/// non-native Bn254 Fq values (e.g. `r_U_secondary` E2-scalar
/// fields) are reinterpreted via `base_as_scalar` / `scalar_as_base`
/// at proof-extraction time — the decider treats them as opaque
/// Bn254 Fr public inputs.
#[derive(Clone, Debug)]
pub struct SectionBPublicInputs {
    /// `hash_secondary` — squeezed by E1 RO over (pp_digest_reinterp,
    /// num_steps, 0, 0, r_U_primary fields, ri_secondary). Native
    /// Poseidon. Equals `compressed_snark.l_u_secondary.X[1]`.
    pub hash_secondary_claimed: Bn254Fr,
    /// `hash_primary` — squeezed by E2 RO (Bn254 Fq, FOREIGN field).
    /// Reinterpreted to Bn254 Fr via `base_as_scalar::<E1>`. Equals
    /// `base_as_scalar::<E1>(compressed_snark.l_u_secondary.X[0])`.
    /// Delegated trick (per SECTION_B_SCOPING.md §2): this is a PI
    /// from the off-circuit `CompressedSNARK::verify`; no in-circuit
    /// non-native Poseidon needed.
    pub hash_primary_reinterp: Bn254Fr,
    /// `vk.pp_digest` — public-parameter digest.
    pub pp_digest: Bn254Fr,
    /// Number of IVC steps (cast to field).
    pub num_steps: Bn254Fr,
    /// `compressed_snark.ri_secondary`.
    pub ri_secondary: Bn254Fr,
    /// `r_U_primary` fields absorbed in the E1 RO via `absorb_in_ro`:
    /// per nova-snark's `R1CSInstance::absorb_in_ro` =
    /// (comm.x, comm.y, X[0], X[1]).
    pub r_U_primary_comm_x: Bn254Fr,
    pub r_U_primary_comm_y: Bn254Fr,
    pub r_U_primary_x0: Bn254Fr,
    pub r_U_primary_x1: Bn254Fr,
    /// Initial state `z0[..]` (IVC arity).
    pub z0: Vec<Bn254Fr>,
    /// Final state `zn[..]` (same arity as z0).
    pub zn: Vec<Bn254Fr>,
}

impl SectionBPublicInputs {
    /// Total public-input count = 9 fixed + |z0| + |zn|. Useful for
    /// downstream Groth16 setup expecting a specific PI count.
    pub fn pi_count(&self) -> usize {
        9 + self.z0.len() + self.zn.len()
    }
}

/// Witness for the recursion decider. Section A (the secondary IPA
/// `ck_hat` MSM) is always present and live. Section B is optional
/// (`section_b`); when `Some`, the circuit allocates the Section B
/// public inputs but **does not yet enforce them** (that's the
/// next iteration per `SECTION_B_SCOPING.md` §7). Section A always
/// satisfies its own binding when consistent.
///
/// Owned, plain data — circuit-agnostic so it can be produced by a
/// future nova-snark proof adapter without touching this module.
#[derive(Clone, Debug)]
pub struct RecursionDeciderCircuit {
    /// IPA opening scalars `s` (Grumpkin scalar field = BN254 Fq).
    /// Non-native in this BN254-Fr circuit.
    pub scalars: Vec<Bn254Fq>,
    /// Commitment-key bases `ck` (Grumpkin points; coords are
    /// BN254-Fr = native).
    pub bases: Vec<Affine<GrumpkinConfig>>,
    /// Pedersen blinding scalar (non-native Fq).
    pub blind: Bn254Fq,
    /// Blinding base `h`.
    pub h: Affine<GrumpkinConfig>,
    /// The claimed `ck_hat` commitment the IPA verifier reconstructs.
    /// Section A enforces the in-circuit MSM equals THIS.
    pub claimed_ck_hat: Projective<GrumpkinConfig>,
    /// Section B public-input bundle. `None` ⇒ Section A only
    /// (no Section B PIs allocated, all existing fixtures /
    /// tests unchanged). `Some` ⇒ PIs allocated but NOT YET
    /// enforced (interface-only — enforcement is the next
    /// iteration per `SECTION_B_SCOPING.md`).
    pub section_b: Option<SectionBPublicInputs>,
    /// Explicit honesty flag: `false` until Sections B-D (the
    /// constant-size hash/NIFS/HyperKZG terms) are wired. A complete
    /// EVM decider requires `true`; Section-A-only instances set
    /// `false` so they cannot be mistaken for a full decider.
    /// Even with `section_b: Some(...)` this stays `false` until
    /// the enforcement lands.
    pub sections_bcd_wired: bool,
}

impl RecursionDeciderCircuit {
    /// Section-A-only constructor (current milestone). `bases` and
    /// `scalars` must be equal length; `claimed_ck_hat` must equal
    /// `Σ scalarsᵢ·basesᵢ + blind·h` for the CS to be satisfiable.
    pub fn section_a_only(
        scalars: Vec<Bn254Fq>,
        bases: Vec<Affine<GrumpkinConfig>>,
        blind: Bn254Fq,
        h: Affine<GrumpkinConfig>,
        claimed_ck_hat: Projective<GrumpkinConfig>,
    ) -> Self {
        Self {
            scalars,
            bases,
            blind,
            h,
            claimed_ck_hat,
            section_b: None,
            sections_bcd_wired: false,
        }
    }

    /// Section-A-plus-Section-B-interface constructor. Allocates
    /// Section B public inputs in `generate_constraints` but DOES
    /// NOT enforce the Poseidon binding yet — enforcement is the
    /// next iteration per `SECTION_B_SCOPING.md` §7 C/D. Use this
    /// to pin the PI layout against downstream Groth16 setup.
    pub fn section_a_with_b_interface(
        scalars: Vec<Bn254Fq>,
        bases: Vec<Affine<GrumpkinConfig>>,
        blind: Bn254Fq,
        h: Affine<GrumpkinConfig>,
        claimed_ck_hat: Projective<GrumpkinConfig>,
        section_b: SectionBPublicInputs,
    ) -> Self {
        Self {
            scalars,
            bases,
            blind,
            h,
            claimed_ck_hat,
            section_b: Some(section_b),
            sections_bcd_wired: false,
        }
    }

    /// Shape-only constructor for Groth16 trusted setup (§7 step 1 of
    /// the audit dossier). Allocates `n_aux` zero scalars + identity
    /// `claimed_ck_hat`, so the CS structure matches a real prover at
    /// the same `bases` length but no specific witness values bind.
    ///
    /// IMPORTANT: the `bases` vector is BAKED INTO THE CIRCUIT as
    /// constants (per `pedersen_msm_grumpkin`'s `GrumpkinVar::constant`).
    /// Therefore setup MUST use the EXACT bases the prover will use —
    /// passing different bases at prove-time produces a circuit with a
    /// different constraint shape that the keys won't fit.
    ///
    /// At zero scalars + zero blind + identity claimed_ck_hat, the CS
    /// is self-consistent (0 + 0 = 0), so the shape can also be used
    /// as a smoke test of the trivial witness.
    pub fn setup_shape(
        bases: Vec<Affine<GrumpkinConfig>>,
        h: Affine<GrumpkinConfig>,
    ) -> Self {
        use ark_std::Zero;
        let n = bases.len();
        Self {
            scalars: vec![Bn254Fq::zero(); n],
            bases,
            blind: Bn254Fq::zero(),
            h,
            claimed_ck_hat: Projective::<GrumpkinConfig>::zero(),
            section_b: None,
            sections_bcd_wired: false,
        }
    }

    /// Section-B-aware setup shape. Allocates the same Section A
    /// shape as `setup_shape` plus all Section B PI slots (with
    /// zero witness values), so Groth16 setup keys the circuit at
    /// the FULL Section A + Section B PI layout.
    ///
    /// `pi_arity` controls the variable-length z0/zn lengths; for
    /// canonical `TrivialIncrementCircuit` this is 1.
    pub fn setup_shape_with_b_interface(
        bases: Vec<Affine<GrumpkinConfig>>,
        h: Affine<GrumpkinConfig>,
        pi_arity: usize,
    ) -> Self {
        use ark_std::Zero;
        let n = bases.len();
        Self {
            scalars: vec![Bn254Fq::zero(); n],
            bases,
            blind: Bn254Fq::zero(),
            h,
            claimed_ck_hat: Projective::<GrumpkinConfig>::zero(),
            section_b: Some(SectionBPublicInputs {
                hash_secondary_claimed: Bn254Fr::zero(),
                hash_primary_reinterp: Bn254Fr::zero(),
                pp_digest: Bn254Fr::zero(),
                num_steps: Bn254Fr::zero(),
                ri_secondary: Bn254Fr::zero(),
                r_U_primary_comm_x: Bn254Fr::zero(),
                r_U_primary_comm_y: Bn254Fr::zero(),
                r_U_primary_x0: Bn254Fr::zero(),
                r_U_primary_x1: Bn254Fr::zero(),
                z0: vec![Bn254Fr::zero(); pi_arity],
                zn: vec![Bn254Fr::zero(); pi_arity],
            }),
            sections_bcd_wired: false,
        }
    }
}

impl ConstraintSynthesizer<Bn254Fr> for RecursionDeciderCircuit {
    fn generate_constraints(
        self,
        cs: ConstraintSystemRef<Bn254Fr>,
    ) -> Result<(), SynthesisError> {
        // ── Section A: secondary IPA `ck_hat` MSM binding [LIVE] ──
        //
        // Witness the non-native scalars; recompute the Pedersen MSM
        // natively (points have BN254-Fr coords); enforce it equals
        // the claimed commitment. This is THE dominant term and the
        // whole reason this circuit escapes the S4b/D.3 blow-up.
        if self.scalars.len() != self.bases.len() {
            // Length mismatch is a malformed witness — same
            // Unsatisfiable contract the crate uses elsewhere.
            return Err(SynthesisError::Unsatisfiable);
        }

        let scalar_vars: Vec<EmulatedFpVar<Bn254Fq, Bn254Fr>> = self
            .scalars
            .iter()
            .map(|s| EmulatedFpVar::new_witness(cs.clone(), || Ok(*s)))
            .collect::<Result<_, _>>()?;
        let blind_var =
            EmulatedFpVar::<Bn254Fq, Bn254Fr>::new_witness(cs.clone(), || Ok(self.blind))?;

        let computed =
            pedersen_msm_grumpkin(&scalar_vars, &self.bases, &blind_var, self.h)?;

        let claimed_var =
            GrumpkinVar::new_witness(cs.clone(), || Ok(self.claimed_ck_hat))?;

        computed.enforce_equal(&claimed_var)?;

        // ── Section B: output-hash binding [INTERFACE WIRED, NOT
        //               ENFORCED YET — see SECTION_B_SCOPING.md §7
        //               for the 3-iteration close plan] ────────────
        //
        // When `section_b` is `Some`, allocate all the Section B
        // public inputs via `new_input` so the Groth16 PI layout is
        // pinned at this milestone (preserves the (e)-1/(e)-2 fixture
        // contract: changes to PI layout require coordinated fixture
        // regeneration). NO enforce_equal calls yet — that's the
        // next iteration. `sections_bcd_wired` stays `false`.
        use ark_r1cs_std::fields::fp::FpVar;
        if let Some(b) = &self.section_b {
            let _hash_sec = FpVar::<Bn254Fr>::new_input(
                cs.clone(), || Ok(b.hash_secondary_claimed),
            )?;
            let _hash_pri = FpVar::<Bn254Fr>::new_input(
                cs.clone(), || Ok(b.hash_primary_reinterp),
            )?;
            let _pp_digest = FpVar::<Bn254Fr>::new_input(
                cs.clone(), || Ok(b.pp_digest),
            )?;
            let _num_steps = FpVar::<Bn254Fr>::new_input(
                cs.clone(), || Ok(b.num_steps),
            )?;
            let _ri_sec = FpVar::<Bn254Fr>::new_input(
                cs.clone(), || Ok(b.ri_secondary),
            )?;
            let _ru_cx = FpVar::<Bn254Fr>::new_input(
                cs.clone(), || Ok(b.r_U_primary_comm_x),
            )?;
            let _ru_cy = FpVar::<Bn254Fr>::new_input(
                cs.clone(), || Ok(b.r_U_primary_comm_y),
            )?;
            let _ru_x0 = FpVar::<Bn254Fr>::new_input(
                cs.clone(), || Ok(b.r_U_primary_x0),
            )?;
            let _ru_x1 = FpVar::<Bn254Fr>::new_input(
                cs.clone(), || Ok(b.r_U_primary_x1),
            )?;
            for z in &b.z0 {
                let _ = FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(*z))?;
            }
            for z in &b.zn {
                let _ = FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(*z))?;
            }
        }

        // ── Section C: NIFS folds + derandomize  [DEFERRED stub] ──
        // ── Section D: primary HyperKZG pairing  [DEFERRED stub] ──
        //
        // Constant-size by source read #3 (see module docs +
        // MAINNET_REMAINING_WORK_FLOW.md). Wired in later increments;
        // `sections_bcd_wired` records their absence so a caller
        // cannot mistake this for a complete decider.

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_ec::{short_weierstrass::SWCurveConfig, CurveGroup};
    use ark_relations::gr1cs::ConstraintSystem;

    /// Build a self-consistent Section-A witness: 3 bases, scalars,
    /// blind, and the correctly-computed native `ck_hat`.
    fn consistent_witness() -> RecursionDeciderCircuit {
        let g = Projective::from(GrumpkinConfig::GENERATOR);
        let g2 = g + g;
        let g3 = g2 + g;
        let h_pt = g * Bn254Fq::from(11u64);
        let bases = vec![
            g.into_affine(),
            g2.into_affine(),
            g3.into_affine(),
        ];
        let scalars = vec![
            Bn254Fq::from(2u64),
            Bn254Fq::from(3u64),
            Bn254Fq::from(4u64),
        ];
        let blind = Bn254Fq::from(5u64);
        let claimed = g * scalars[0]
            + g2 * scalars[1]
            + g3 * scalars[2]
            + h_pt * blind;
        RecursionDeciderCircuit::section_a_only(
            scalars,
            bases,
            blind,
            h_pt.into_affine(),
            claimed,
        )
    }

    /// POSITIVE: a correct `claimed_ck_hat` ⇒ CS satisfied. Proves
    /// Section A's native MSM matches the out-of-circuit commitment.
    #[test]
    fn section_a_correct_commitment_satisfies_cs() {
        let circuit = consistent_witness();
        assert!(!circuit.sections_bcd_wired, "Section-A-only ⇒ flag false");
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        circuit.generate_constraints(cs.clone()).expect("synthesis");
        assert!(
            cs.is_satisfied().expect("is_satisfied"),
            "correct ck_hat must satisfy the Section A binding"
        );
    }

    /// NEGATIVE: a wrong `claimed_ck_hat` ⇒ CS UNSATISFIED. Proves
    /// the binding actually constrains (not a vacuous gate) — the
    /// exact B-1 hazard (`dummy()` vacuity) this must avoid.
    #[test]
    fn section_a_wrong_commitment_breaks_cs() {
        let mut circuit = consistent_witness();
        // Tamper: add G to the claimed commitment.
        circuit.claimed_ck_hat += Projective::from(GrumpkinConfig::GENERATOR);
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        circuit.generate_constraints(cs.clone()).expect("synthesis");
        assert!(
            !cs.is_satisfied().expect("is_satisfied"),
            "a wrong ck_hat MUST break the CS — binding must be non-vacuous"
        );
    }

    /// Malformed witness (scalars/bases length mismatch) ⇒
    /// Unsatisfiable, the crate-wide contract.
    #[test]
    fn section_a_length_mismatch_is_unsatisfiable() {
        let mut circuit = consistent_witness();
        circuit.scalars.pop();
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        let r = circuit.generate_constraints(cs.clone());
        assert!(
            matches!(r, Err(SynthesisError::Unsatisfiable)),
            "length mismatch must map to SynthesisError::Unsatisfiable"
        );
    }

    /// Section B INTERFACE smoke (per SECTION_B_SCOPING.md §7 A-B):
    /// allocates Section A + Section B PIs; counts public inputs;
    /// pins that adding Section B's interface (without enforcement)
    /// does NOT make the CS unsatisfied (Section A's binding is the
    /// only enforced gate; Section B PIs are decorative until
    /// enforcement lands in the next iteration). Also counts the
    /// expected total PIs (9 fixed + |z0| + |zn|) and the extra
    /// constraint cost over Section-A-only (should be ~0).
    #[test]
    fn section_b_interface_wiring_compiles_and_pis_count() {
        use ark_ec::short_weierstrass::SWCurveConfig;
        use ark_ec::CurveGroup;

        // Build a consistent Section A witness (so the existing
        // binding is satisfied).
        let circuit_a = consistent_witness();
        let cs_a = ConstraintSystem::<Bn254Fr>::new_ref();
        circuit_a
            .clone()
            .generate_constraints(cs_a.clone())
            .expect("Section A synthesis");
        let n_inst_a = cs_a.num_instance_variables();
        let n_cons_a = cs_a.num_constraints();

        // Now wrap the same Section A witness with a Section B
        // interface (arity-2 z0/zn, arbitrary values).
        let b = SectionBPublicInputs {
            hash_secondary_claimed: Bn254Fr::from(101u64),
            hash_primary_reinterp: Bn254Fr::from(102u64),
            pp_digest: Bn254Fr::from(103u64),
            num_steps: Bn254Fr::from(7u64),
            ri_secondary: Bn254Fr::from(104u64),
            r_U_primary_comm_x: Bn254Fr::from(105u64),
            r_U_primary_comm_y: Bn254Fr::from(106u64),
            r_U_primary_x0: Bn254Fr::from(107u64),
            r_U_primary_x1: Bn254Fr::from(108u64),
            z0: vec![Bn254Fr::from(200u64), Bn254Fr::from(201u64)],
            zn: vec![Bn254Fr::from(300u64), Bn254Fr::from(301u64)],
        };
        let expected_b_pi_count = b.pi_count();
        assert_eq!(
            expected_b_pi_count,
            9 + 2 + 2,
            "Section B PI count formula = 9 + |z0| + |zn|"
        );

        let circuit_ab = RecursionDeciderCircuit::section_a_with_b_interface(
            circuit_a.scalars,
            circuit_a.bases,
            circuit_a.blind,
            circuit_a.h,
            circuit_a.claimed_ck_hat,
            b,
        );
        assert!(
            !circuit_ab.sections_bcd_wired,
            "honesty flag stays false until enforcement lands"
        );

        let cs_ab = ConstraintSystem::<Bn254Fr>::new_ref();
        circuit_ab
            .generate_constraints(cs_ab.clone())
            .expect("Section A + B-interface synthesis");

        // Section A binding still satisfied (B has no enforcement yet).
        assert!(
            cs_ab.is_satisfied().expect("is_satisfied"),
            "Section B interface (no enforcement) must NOT break Section A"
        );

        // Public inputs grew by exactly the Section B PI count.
        let n_inst_ab = cs_ab.num_instance_variables();
        // cs.num_instance_variables includes the constant-1 input, so
        // the delta is the user-allocated PIs only.
        assert_eq!(
            n_inst_ab - n_inst_a,
            expected_b_pi_count,
            "PI delta must equal SectionBPublicInputs::pi_count()"
        );

        // Constraint count grew by negligible amount (new_input alone
        // adds no R1CS constraints — just instance allocations).
        let n_cons_ab = cs_ab.num_constraints();
        assert_eq!(
            n_cons_ab, n_cons_a,
            "Section B interface allocation must add 0 constraints"
        );
    }

    /// (d)-3 (§7 step 3 EXTENDED): cs.num_constraints() of the FULL
    /// `RecursionDeciderCircuit::setup_shape` at multiple n. Validates
    /// the (d)-1 linear-fit prediction (per-base 2,533, intercept
    /// 2,521) against the actual wrapping circuit — checks for any
    /// circuit-level overhead the gadget-only probe couldn't see.
    /// Fast (synthetic doubling-chain bases, no PP setup).
    #[test]
    fn setup_shape_cons_scaling_validates_d1_prediction() {
        use ark_ec::short_weierstrass::SWCurveConfig;
        use ark_ec::CurveGroup;

        // Synthetic doubling-chain bases — distinct, real Grumpkin
        // points; shape probe doesn't need real PP.
        let g = Projective::<GrumpkinConfig>::from(GrumpkinConfig::GENERATOR);
        let h_pt = g * Bn254Fq::from(7u64);
        let h_aff = h_pt.into_affine();

        let measure = |n: usize| -> usize {
            let mut bases = Vec::with_capacity(n);
            let mut cur = g;
            for _ in 0..n {
                bases.push(cur.into_affine());
                cur += g;
            }
            let circuit = RecursionDeciderCircuit::setup_shape(bases, h_aff);
            let cs = ConstraintSystem::<Bn254Fr>::new_ref();
            circuit.generate_constraints(cs.clone()).expect("synthesis");
            cs.num_constraints()
        };

        // Scan n; report each measurement.
        let ns: [usize; 5] = [4, 16, 64, 256, 1024];
        let cs_counts: Vec<(usize, usize)> =
            ns.iter().map(|&n| (n, measure(n))).collect();
        for (n, c) in &cs_counts {
            eprintln!(
                "DECIDER_CONS n={n} cons={c} per_base={}",
                if *n > 0 { c / n } else { 0 }
            );
        }

        // Linear fit on the upper-end pair to avoid small-n overhead
        // distortion. per_base = (c_1024 - c_64) / (1024 - 64).
        let c_small = cs_counts[2].1; // n=64
        let c_big = cs_counts[4].1;   // n=1024
        let per_base = (c_big - c_small) / (1024 - 64);
        let intercept = c_big - per_base * 1024;

        // (d)-1 model: per_base=2,533, intercept ≈ 2,521 (the blind
        // scalar-mul) measured at gadget level.
        eprintln!(
            "DECIDER_FIT per_base={per_base} intercept={intercept} \
             D1_PRED_per_base=2533"
        );

        // Extrapolation to n_aux=16,384.
        let pred_at_full = intercept + per_base * 16_384;
        eprintln!(
            "DECIDER_PRED_at_n_aux_16384 cons={pred_at_full} ~{}M",
            pred_at_full / 1_000_000
        );

        // Sanity: per-base must land near (d)-1's 2,533 — allow ±15%
        // for circuit-wrapping overhead.
        let lo = 2533 * 85 / 100;
        let hi = 2533 * 115 / 100;
        assert!(
            per_base >= lo && per_base <= hi,
            "decider per-base cons {per_base} outside ±15% of (d)-1 model 2533 \
             ([{lo}, {hi}]) — wrapping overhead unexpected"
        );

        // FALSIFIER: predicted cons at n_aux=16,384 must stay < 1e8
        // (Groth16 memory wall on 128 GB at ~1.2 GB / 10M cons).
        assert!(
            pred_at_full < 100_000_000,
            "FALSIFIER TRIPPED: predicted decider cons {pred_at_full} ≥ 1e8 \
             — Groth16 setup may not fit Mini cluster RAM"
        );
    }

    /// INCREMENT-2 FINISH — real witness-assembly pipeline on REAL
    /// data: real `pp` → `extract_secondary_ck` (real Grumpkin
    /// bases) → `ipa_s_vector` (real tensor structure) → real
    /// `ck_hat` → `RecursionDeciderCircuit` Section A. Proves the
    /// end-to-end plumbing is correct on real curve points, not toy
    /// data. Run at a real-bases but **tractable** n = 256 (≈ 0.65M
    /// cons); the full-n (~16384, ~41M cons) synthesis is the
    /// deliberately-scheduled heavy step, NOT silently skipped.
    /// `#[ignore]`: needs `canonical_public_params` (pp setup, secs).
    #[test]
    #[ignore = "increment-2: real pp ck_secondary extract + real tensor (Mini)"]
    fn section_a_real_bases_real_tensor_pipeline() {
        use crate::ipa_s_tensor::ipa_s_vector;
        use crate::recursive_snark_fixture::canonical_public_params;
        use crate::s4_secondary_extract::extract_secondary_ck;
        use ark_std::Zero;

        let pp = canonical_public_params().expect("canonical pp");
        let pp_json = serde_json::to_value(&pp).expect("pp to_value");
        let (ck_full, h) =
            extract_secondary_ck(&pp_json).expect("extract real ck_secondary");
        assert!(
            ck_full.len() >= 256,
            "real ck_secondary must have ≥256 bases, got {}",
            ck_full.len()
        );

        // Real bases, real tensor structure, tractable n = 2^8.
        let rounds = 8usize;
        let n = 1usize << rounds;
        let ck: Vec<Affine<GrumpkinConfig>> = ck_full[..n].to_vec();
        let r: Vec<Bn254Fq> =
            (0..rounds).map(|i| Bn254Fq::from(i as u64 + 2)).collect();
        let s = ipa_s_vector(&r);
        assert_eq!(s.len(), n, "tensor s must have length n");
        let blind = Bn254Fq::from(9u64);

        // Out-of-circuit real ck_hat = Σ sᵢ·ckᵢ + blind·h.
        let claimed = ck
            .iter()
            .zip(s.iter())
            .fold(Projective::<GrumpkinConfig>::zero(), |acc, (c, si)| {
                acc + Projective::from(*c) * *si
            })
            + Projective::from(h) * blind;

        let circuit = RecursionDeciderCircuit::section_a_only(
            s.clone(),
            ck.clone(),
            blind,
            h,
            claimed,
        );
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        circuit.generate_constraints(cs.clone()).expect("synthesis");
        assert!(
            cs.is_satisfied().expect("is_satisfied"),
            "real-bases real-tensor Section A must satisfy CS at n={n}"
        );

        // Non-vacuous on real data too: tamper ⇒ UNSAT.
        let bad = RecursionDeciderCircuit::section_a_only(
            s,
            ck,
            blind,
            h,
            claimed + Projective::from(GrumpkinConfig::GENERATOR),
        );
        let cs_bad = ConstraintSystem::<Bn254Fr>::new_ref();
        bad.generate_constraints(cs_bad.clone()).expect("synthesis");
        assert!(
            !cs_bad.is_satisfied().expect("is_satisfied"),
            "wrong real ck_hat MUST break CS — binding non-vacuous on real data"
        );
    }
}
