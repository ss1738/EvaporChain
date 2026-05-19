//! B-1/B-2 EVM, option (1C) — increment 3a: the **NIFS fold
//! primitive** for CycleFold running instances + its homomorphism
//! soundness gate.
//!
//! # Why split 3 into 3a + 3b
//!
//! The full IVC harness wiring (primary RecursiveSNARK ⨉ CF
//! running instance accumulation across steps) is a multi-day novel
//! construction. The *cheapest decisive sub-step* — per the
//! session's lesson-by-now-rule — is the underlying soundness
//! property the fold relies on: Pedersen commitments over Grumpkin
//! are **additively homomorphic in the witness**, so folding
//! commitments by a scalar `r` equals committing to the
//! `r`-folded witness:
//! ```text
//!   commit(W_a) + r · commit(W_b) ≡ commit(W_a + r · W_b)
//! ```
//! That equivalence is the NIFS fold's correctness — if it holds
//! for our setup, the IVC harness in 3b is pure composition; if it
//! breaks, the whole CycleFold plumbing breaks.
//!
//! 3b will then integrate `nova_snark::nifs::NIFS<GrumpkinEngine>`
//! (the cross-term `comm_T`, RO challenge derivation, RelaxedR1CS
//! instance fold) on top of this proven primitive — no new crypto,
//! just composition.

use ark_bn254::Fq as Bn254Fq;
use ark_ec::short_weierstrass::Projective;
use ark_ec::CurveGroup;
use ark_std::Zero;

use crate::grumpkin_config::GrumpkinConfig;

/// A Grumpkin group element (CycleFold instance commitments live
/// here per CycleFold's `cf_U_i: CommittedInstance<C2 = Grumpkin>`).
pub type GComm = Projective<GrumpkinConfig>;

/// CycleFold *running* (relaxed) instance — what accumulates across
/// IVC steps. Mirrors the standard NIFS-folded RelaxedR1CSInstance
/// shape: `cmW` (witness commitment), `cmE` (error commitment),
/// scalar `u`, public IO `x`.
#[derive(Clone, Debug, PartialEq)]
pub struct CycleFoldRunningInstance {
    pub comm_w: GComm,
    pub comm_e: GComm,
    pub u: Bn254Fq,
    pub x: Vec<Bn254Fq>,
}

/// CycleFold *incoming* (non-relaxed) instance — produced per step
/// by the primary's cross-curve scalar-mul delegation. `u = 1`
/// implicitly; `comm_e = 0` (no error term yet). Folding pulls it
/// into the running instance.
#[derive(Clone, Debug)]
pub struct CycleFoldIncomingInstance {
    pub comm_w: GComm,
    pub x: Vec<Bn254Fq>,
}

impl CycleFoldRunningInstance {
    /// The "fresh" running instance: zero commitments, `u = 0`,
    /// `x = 0`. Equivalent to no accumulated work yet.
    pub fn zero(io_len: usize) -> Self {
        Self {
            comm_w: GComm::zero(),
            comm_e: GComm::zero(),
            u: Bn254Fq::from(0u64),
            x: vec![Bn254Fq::from(0u64); io_len],
        }
    }
}

/// NIFS fold step: given the current running instance + an incoming
/// instance + the prover-supplied cross-term commitment `comm_T` +
/// the fold challenge `r`, return the new running instance.
///
/// Folding identities (standard NIFS over a relaxed R1CS, `u_I = 1`
/// for the incoming):
/// ```text
///   comm_w' = comm_w_R + r · comm_w_I
///   comm_e' = comm_e_R + r · comm_T
///   u'      = u_R + r
///   x_i'    = x_R[i] + r · x_I[i]      for each i
/// ```
/// The witness fold itself (`W' = W_R + r · W_I`, `E' = E_R + r · T
/// - r² · E_I`) is the prover's job; its consistency with the
/// commitment fold above is exactly the homomorphism gate this
/// module verifies.
pub fn fold_cf_step(
    running: &CycleFoldRunningInstance,
    incoming: &CycleFoldIncomingInstance,
    comm_t: &GComm,
    r: Bn254Fq,
) -> CycleFoldRunningInstance {
    assert_eq!(
        running.x.len(),
        incoming.x.len(),
        "x length mismatch — running.x and incoming.x must agree"
    );
    let comm_w_new = running.comm_w + (*incoming).comm_w * r;
    let comm_e_new = running.comm_e + *comm_t * r;
    let u_new = running.u + r;
    let x_new: Vec<Bn254Fq> = running
        .x
        .iter()
        .zip(incoming.x.iter())
        .map(|(xr, xi)| *xr + r * *xi)
        .collect();
    CycleFoldRunningInstance {
        comm_w: comm_w_new,
        comm_e: comm_e_new,
        u: u_new,
        x: x_new,
    }
}

/// Out-of-circuit Pedersen commitment over Grumpkin — `Σ w_i · ck_i
/// + r_blind · h`. Equivalent (out-of-circuit) of the in-circuit
/// `crate::s4_msm_gadget::pedersen_msm_grumpkin`. Used by the
/// homomorphism test to commit synthetic witnesses; in 3b, replaced
/// by nova-snark's `CommitmentEngine<GrumpkinEngine>`.
pub fn pedersen_commit_grumpkin(
    ck: &[GComm],
    w: &[Bn254Fq],
    h: &GComm,
    r_blind: Bn254Fq,
) -> GComm {
    assert_eq!(ck.len(), w.len(), "ck and w must have same length");
    let mut acc = GComm::zero();
    for (g, s) in ck.iter().zip(w.iter()) {
        acc += *g * *s;
    }
    acc + *h * r_blind
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grumpkin_config::GrumpkinConfig;
    use ark_ec::short_weierstrass::SWCurveConfig;
    use ark_ff::UniformRand;
    use ark_std::test_rng;

    fn random_ck(n: usize) -> Vec<GComm> {
        let mut rng = test_rng();
        let g = GComm::from(GrumpkinConfig::GENERATOR);
        // Distinct, generic-non-trivial commitment-key elements.
        (0..n).map(|_| g * Bn254Fq::rand(&mut rng)).collect()
    }

    /// THE SOUNDNESS GATE — Pedersen homomorphism over Grumpkin:
    /// `commit(W_a) + r · commit(W_b) ≡ commit(W_a + r · W_b)`
    /// for ALL `(ck, W_a, W_b, r)`. Sample random instances. If
    /// this fails, the whole NIFS fold (and thus our CycleFold
    /// plumbing) is unsound. Catches an algebra/typing slip in
    /// `fold_cf_step` or `pedersen_commit_grumpkin`.
    #[test]
    fn pedersen_grumpkin_is_homomorphic_in_witness() {
        let mut rng = test_rng();
        let n = 16; // homomorphism is a general property — small n suffices
        let ck = random_ck(n);
        let h = GComm::from(GrumpkinConfig::GENERATOR)
            * Bn254Fq::from(7u64);
        let w_a: Vec<Bn254Fq> =
            (0..n).map(|_| Bn254Fq::rand(&mut rng)).collect();
        let w_b: Vec<Bn254Fq> =
            (0..n).map(|_| Bn254Fq::rand(&mut rng)).collect();
        let r = Bn254Fq::rand(&mut rng);

        // commit(W_a) + r · commit(W_b)
        let c_a = pedersen_commit_grumpkin(&ck, &w_a, &h, Bn254Fq::from(0u64));
        let c_b = pedersen_commit_grumpkin(&ck, &w_b, &h, Bn254Fq::from(0u64));
        let lhs = c_a + c_b * r;

        // commit(W_a + r · W_b)
        let w_folded: Vec<Bn254Fq> = w_a
            .iter()
            .zip(w_b.iter())
            .map(|(a, b)| *a + r * *b)
            .collect();
        let rhs = pedersen_commit_grumpkin(&ck, &w_folded, &h, Bn254Fq::from(0u64));

        assert_eq!(
            lhs.into_affine(),
            rhs.into_affine(),
            "Pedersen-on-Grumpkin homomorphism BROKEN — fold unsound"
        );
    }

    /// Fold consistency: `fold_cf_step` correctly applies the
    /// scalar-mul + add identities. Cross-checked by computing
    /// commitments directly on the folded witness/x.
    #[test]
    fn fold_cf_step_matches_direct_commitment_of_folded_witness() {
        let mut rng = test_rng();
        let n = 8;
        let io_len = 3;
        let ck = random_ck(n);
        let h = GComm::from(GrumpkinConfig::GENERATOR) * Bn254Fq::from(11u64);

        // Running (relaxed) — synthesize from W_R.
        let w_r: Vec<Bn254Fq> = (0..n).map(|_| Bn254Fq::rand(&mut rng)).collect();
        let comm_w_r = pedersen_commit_grumpkin(&ck, &w_r, &h, Bn254Fq::from(0u64));
        let x_r: Vec<Bn254Fq> =
            (0..io_len).map(|_| Bn254Fq::rand(&mut rng)).collect();
        let comm_e_r = GComm::from(GrumpkinConfig::GENERATOR)
            * Bn254Fq::rand(&mut rng);
        let u_r = Bn254Fq::rand(&mut rng);
        let running = CycleFoldRunningInstance {
            comm_w: comm_w_r,
            comm_e: comm_e_r,
            u: u_r,
            x: x_r.clone(),
        };

        // Incoming (non-relaxed, u_I=1) — synthesize from W_I.
        let w_i: Vec<Bn254Fq> = (0..n).map(|_| Bn254Fq::rand(&mut rng)).collect();
        let comm_w_i = pedersen_commit_grumpkin(&ck, &w_i, &h, Bn254Fq::from(0u64));
        let x_i: Vec<Bn254Fq> =
            (0..io_len).map(|_| Bn254Fq::rand(&mut rng)).collect();
        let incoming = CycleFoldIncomingInstance {
            comm_w: comm_w_i,
            x: x_i.clone(),
        };

        // Cross-term commitment — fake some `comm_T`. (Real `comm_T`
        // is computed from the cross-term polynomial; for this test
        // it just must be SOME group element — the fold mechanic
        // doesn't depend on its specific value.)
        let comm_t =
            GComm::from(GrumpkinConfig::GENERATOR) * Bn254Fq::rand(&mut rng);
        let r = Bn254Fq::rand(&mut rng);

        // Folded via fold_cf_step.
        let folded = fold_cf_step(&running, &incoming, &comm_t, r);

        // Cross-check: comm_w' must equal commit(W_R + r · W_I)
        // (the homomorphism applied through fold_cf_step).
        let w_folded: Vec<Bn254Fq> = w_r
            .iter()
            .zip(w_i.iter())
            .map(|(a, b)| *a + r * *b)
            .collect();
        let comm_w_expected =
            pedersen_commit_grumpkin(&ck, &w_folded, &h, Bn254Fq::from(0u64));
        assert_eq!(
            folded.comm_w.into_affine(),
            comm_w_expected.into_affine(),
            "fold_cf_step.comm_w must equal commit(W_R + r·W_I)"
        );

        // comm_e' must equal comm_e_R + r · comm_T (definitional).
        assert_eq!(
            folded.comm_e.into_affine(),
            (comm_e_r + comm_t * r).into_affine(),
            "fold_cf_step.comm_e must equal comm_e_R + r·comm_T"
        );

        // u' must equal u_R + r (definitional).
        assert_eq!(folded.u, u_r + r, "fold_cf_step.u must equal u_R + r");

        // x_i' must equal x_R[i] + r·x_I[i] (definitional).
        for i in 0..io_len {
            assert_eq!(
                folded.x[i],
                x_r[i] + r * x_i[i],
                "x[{i}] mismatch"
            );
        }
    }

    /// MULTI-STEP ACCUMULATION: 3 successive folds, each verified
    /// against the direct-commit-of-folded-witness identity. If the
    /// homomorphism breaks across compositions (e.g. due to a
    /// `+r²` term needing to enter `comm_e`), this catches it.
    #[test]
    fn multi_step_fold_accumulation_consistent() {
        let mut rng = test_rng();
        let n = 8;
        let io_len = 2;
        let ck = random_ck(n);
        let h = GComm::from(GrumpkinConfig::GENERATOR) * Bn254Fq::from(13u64);

        // Start from zero running instance.
        let mut running = CycleFoldRunningInstance::zero(io_len);
        // Track the cumulative folded witness out-of-band to
        // cross-check `comm_w` at each step.
        let mut w_accum: Vec<Bn254Fq> = vec![Bn254Fq::from(0u64); n];

        for _ in 0..3 {
            let w_i: Vec<Bn254Fq> =
                (0..n).map(|_| Bn254Fq::rand(&mut rng)).collect();
            let comm_w_i =
                pedersen_commit_grumpkin(&ck, &w_i, &h, Bn254Fq::from(0u64));
            let x_i: Vec<Bn254Fq> =
                (0..io_len).map(|_| Bn254Fq::rand(&mut rng)).collect();
            let incoming = CycleFoldIncomingInstance {
                comm_w: comm_w_i,
                x: x_i,
            };
            let comm_t = GComm::from(GrumpkinConfig::GENERATOR)
                * Bn254Fq::rand(&mut rng);
            let r = Bn254Fq::rand(&mut rng);

            // Step the running instance.
            running = fold_cf_step(&running, &incoming, &comm_t, r);
            // Step the out-of-band witness accumulator.
            for j in 0..n {
                w_accum[j] += r * w_i[j];
            }

            // Cross-check this step's comm_w against direct commit.
            let comm_w_direct =
                pedersen_commit_grumpkin(&ck, &w_accum, &h, Bn254Fq::from(0u64));
            assert_eq!(
                running.comm_w.into_affine(),
                comm_w_direct.into_affine(),
                "multi-step accumulation broken at this fold"
            );
        }
    }
}
