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
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};

use crate::grumpkin_config::GrumpkinConfig;
use crate::s4_msm_gadget::{pedersen_msm_grumpkin, GrumpkinVar};

/// Witness for the recursion decider's Section A (the secondary IPA
/// `ck_hat` MSM). Owned, plain data — circuit-agnostic so it can be
/// produced by a future nova-snark proof adapter without touching
/// this module.
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
    /// Explicit honesty flag: `false` until Sections B-D (the
    /// constant-size hash/NIFS/HyperKZG terms) are wired. A complete
    /// EVM decider requires `true`; Section-A-only instances set
    /// `false` so they cannot be mistaken for a full decider.
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

        // ── Section B: Neptune hash anchors [DEFERRED stub] ───────
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
    use ark_relations::r1cs::ConstraintSystem;

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
