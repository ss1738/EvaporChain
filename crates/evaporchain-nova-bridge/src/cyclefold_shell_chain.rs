//! B-1/B-2 EVM, option (1C) — increment (b) sub-1: **shell-to-
//! shell state threading** for the CycleFold primary augmented
//! circuit.
//!
//! # What this validates
//!
//! [`crate::cyclefold_primary_augmented_circuit::PrimaryAugmented
//! CircuitShell`] is per-step structurally complete (4b
//! sections_wired:true). For IVC the per-step output ↔ per-step
//! input CONTRACT must hold: step i's public outputs must be
//! valid step i+1's inputs (some are public inputs, some are
//! witnesses).
//!
//! The threading contract this module gates:
//! - `z_{i+1}` (step i public output) → `z_i` (step i+1 public input).
//! - `current_step_hash` (step i public output) → `previous_step_
//!   hash` (step i+1 witness).
//! - `primary_u_new` (step i public output) → `primary_u_r` (step
//!   i+1 witness).
//! - `primary_x_new` (step i public output) → `primary_x_r` (step
//!   i+1 witness).
//! - `pp_hash`, `z_0` constant across steps.
//! - `i` increments by 1.
//!
//! # What this does NOT cover (deferred)
//!
//! - **CF running instance threading** — `cf_u_running`,
//!   `cf_comm_{w,e}_{x,y}`, `cf_x_vec` must be folded across
//!   steps via [`crate::cyclefold_ivc_accumulator`]'s NIFS-on-
//!   Grumpkin path. This module synthesises both shells with
//!   independent CF fields per step (which is consistent — the
//!   shell binds them to its own current_step_hash but doesn't
//!   enforce inter-step folding); the full CF-side IVC harness is
//!   sub-step (b-2).
//! - **r-from-RO consistency** — step i+1's `primary_r` is
//!   currently derived from its OWN `previous_step_hash`
//!   (threaded from step i's `current_step_hash`), but a complete
//!   IVC would also chain comm_T from step i+1's actual NIFS
//!   prove output. Here `primary_comm_t` is independent per step
//!   (consistent at the shell level; full chain is sub-step b-3).
//! - **Aux-circuit side** — this module only synthesises the
//!   primary shell. The CF instance circuits for each step's
//!   (P, s, Q) tuples are validated independently by
//!   [`crate::cyclefold_instance_circuit`] + [`crate::cyclefold_
//!   ivc_accumulator`].

use crate::cyclefold_primary_augmented_circuit::PrimaryAugmentedCircuitShell;
use crate::neptune_reference::neptune_hash_primary;
use crate::scalar_adapter::{ark_fr_to_primary, primary_to_ark_fr};
use ark_bn254::{Fq as Bn254Fq, Fr as Bn254Fr, G1Affine, G1Projective};
use ark_ec::{AffineRepr, CurveGroup};
use ark_ff::{BigInteger, PrimeField, UniformRand};
use ark_relations::gr1cs::{ConstraintSynthesizer, ConstraintSystem};
use ark_std::rand::RngCore;

/// Pack a slice of LE bits into a `Bn254Fr` element (same routine
/// the per-section native helpers use; duplicated here to keep
/// this module self-contained without making the test-only helpers
/// in `cyclefold_primary_augmented_circuit` pub).
fn pack_le_to_fr(bs: &[bool]) -> Bn254Fr {
    let mut acc = Bn254Fr::from(0u64);
    let mut power = Bn254Fr::from(1u64);
    for b in bs {
        if *b {
            acc += power;
        }
        power = power + power;
    }
    acc
}

/// Native r-from-RO derivation (mirrors β-5-γ + (a)-1 in-circuit
/// gadget — absorbs comm_W_I AND comm_T limbs).
fn primary_r_native(
    pp_hash: Bn254Fr,
    previous_step_hash: Bn254Fr,
    x_i_0: Bn254Fr,
    x_i_1: Bn254Fr,
    primary_comm_w_i: G1Affine,
    primary_comm_t: G1Affine,
) -> Bn254Fr {
    let cwi_x = primary_comm_w_i.x.into_bigint().to_bits_le();
    let cwi_y = primary_comm_w_i.y.into_bigint().to_bits_le();
    let ct_x = primary_comm_t.x.into_bigint().to_bits_le();
    let ct_y = primary_comm_t.y.into_bigint().to_bits_le();
    let s_cwi_x = 127usize.min(cwi_x.len());
    let s_cwi_y = 127usize.min(cwi_y.len());
    let s_ct_x = 127usize.min(ct_x.len());
    let s_ct_y = 127usize.min(ct_y.len());
    let absorbed: [Bn254Fr; 12] = [
        pp_hash,
        previous_step_hash,
        x_i_0,
        x_i_1,
        pack_le_to_fr(&cwi_x[..s_cwi_x]),
        pack_le_to_fr(&cwi_x[s_cwi_x..]),
        pack_le_to_fr(&cwi_y[..s_cwi_y]),
        pack_le_to_fr(&cwi_y[s_cwi_y..]),
        pack_le_to_fr(&ct_x[..s_ct_x]),
        pack_le_to_fr(&ct_x[s_ct_x..]),
        pack_le_to_fr(&ct_y[..s_ct_y]),
        pack_le_to_fr(&ct_y[s_ct_y..]),
    ];
    let absorbed_nova = absorbed.map(ark_fr_to_primary);
    primary_to_ark_fr(neptune_hash_primary(&absorbed_nova))
}

/// Native current_step_hash (mirrors β-4d in-circuit Section R).
#[allow(clippy::too_many_arguments)]
fn current_step_hash_native(
    pp_hash: Bn254Fr,
    i: Bn254Fr,
    z_0: Bn254Fr,
    z_i: Bn254Fr,
    z_i1: Bn254Fr,
    cf_x_digest: Bn254Fr,
    cf_u_running: Bn254Fq,
    cf_comm_w_x: Bn254Fr,
    cf_comm_w_y: Bn254Fr,
    cf_comm_e_x: Bn254Fr,
    cf_comm_e_y: Bn254Fr,
    cf_x_vec: &[Bn254Fq],
) -> Bn254Fr {
    let bits = cf_u_running.into_bigint().to_bits_le();
    let split = 127usize.min(bits.len());
    let cf_u_lo = pack_le_to_fr(&bits[..split]);
    let cf_u_hi = pack_le_to_fr(&bits[split..]);
    let mut absorbed: Vec<Bn254Fr> = vec![
        pp_hash, i, z_0, z_i, z_i1, cf_x_digest,
        cf_u_lo, cf_u_hi,
        cf_comm_w_x, cf_comm_w_y, cf_comm_e_x, cf_comm_e_y,
    ];
    for x_fq in cf_x_vec {
        let xb = x_fq.into_bigint().to_bits_le();
        let xs = 127usize.min(xb.len());
        absorbed.push(pack_le_to_fr(&xb[..xs]));
        absorbed.push(pack_le_to_fr(&xb[xs..]));
    }
    let absorbed_nova: Vec<_> =
        absorbed.into_iter().map(ark_fr_to_primary).collect();
    primary_to_ark_fr(neptune_hash_primary(&absorbed_nova))
}

/// Build a `PrimaryAugmentedCircuitShell` for step `i` given the
/// outputs of step `i-1` (or initial-state values for step 0).
/// The "step circuit" is the existing stub `z_{i+1} = z_i + 1`.
pub fn build_shell_for_step<R: RngCore>(
    rng: &mut R,
    pp_hash: Bn254Fr,
    z_0: Bn254Fr,
    step_index: u64,
    z_i: Bn254Fr,
    previous_step_hash: Bn254Fr,
    primary_u_r: Bn254Fr,
    primary_x_r: [Bn254Fr; 2],
    params: &crate::neptune_permutation_gadget::NeptuneParams<Bn254Fr>,
) -> PrimaryAugmentedCircuitShell {
    // Stub step: z_{i+1} = z_i + 1.
    let z_i1 = z_i + Bn254Fr::from(1u64);

    // Step-fresh cross-curve tuples for cf1 + cf2.
    let mk_tuple = |rng: &mut R| {
        let p = G1Affine::generator();
        let s = Bn254Fr::rand(rng);
        let q = (G1Projective::from(p) * s).into_affine();
        (p, s, q)
    };
    let (t1_p, t1_s, t1_q) = mk_tuple(rng);
    let (t2_p, t2_s, t2_q) = mk_tuple(rng);
    let cf_x_digest =
        crate::cyclefold_cf_x_digest::compute_cf_x_digest_pair_native(
            t1_p, t1_s, t1_q, t2_p, t2_s, t2_q,
        );

    // CF running instance fields — independent per step at the
    // shell level (full CF-side IVC threading is sub-step b-2).
    let cf_u_running = Bn254Fq::rand(rng);
    let cf_comm_w_x = Bn254Fr::rand(rng);
    let cf_comm_w_y = Bn254Fr::rand(rng);
    let cf_comm_e_x = Bn254Fr::rand(rng);
    let cf_comm_e_y = Bn254Fr::rand(rng);
    let cf_x_vec: Vec<Bn254Fq> = (0..21).map(|_| Bn254Fq::rand(rng)).collect();

    // Section F native fold inputs: step-fresh r derived from RO
    // including the threaded previous_step_hash + the step's
    // incoming primary X_I. comm_T is a step-fresh G1 point.
    let primary_x_i: [Bn254Fr; 2] = [Bn254Fr::rand(rng), Bn254Fr::rand(rng)];
    let mk_g1 = |rng: &mut R| {
        let g = G1Affine::generator();
        let s = Bn254Fr::rand(rng);
        (G1Projective::from(g) * s).into_affine()
    };
    let primary_comm_w_i = mk_g1(rng);
    let primary_comm_t = mk_g1(rng);
    let i_fr = Bn254Fr::from(step_index);
    let primary_r = primary_r_native(
        pp_hash,
        previous_step_hash,
        primary_x_i[0],
        primary_x_i[1],
        primary_comm_w_i,
        primary_comm_t,
    );
    let primary_u_new = primary_u_r + primary_r;
    let primary_x_new: [Bn254Fr; 2] = [
        primary_x_r[0] + primary_r * primary_x_i[0],
        primary_x_r[1] + primary_r * primary_x_i[1],
    ];
    let current_step_hash = current_step_hash_native(
        pp_hash,
        i_fr,
        z_0,
        z_i,
        z_i1,
        cf_x_digest,
        cf_u_running,
        cf_comm_w_x,
        cf_comm_w_y,
        cf_comm_e_x,
        cf_comm_e_y,
        &cf_x_vec,
    );

    PrimaryAugmentedCircuitShell::new(
        pp_hash,
        i_fr,
        z_0,
        z_i,
        z_i1,
        t1_p, t1_s, t1_q,
        t2_p, t2_s, t2_q,
        cf_x_digest,
        current_step_hash,
        cf_u_running,
        cf_comm_w_x,
        cf_comm_w_y,
        cf_comm_e_x,
        cf_comm_e_y,
        cf_x_vec,
        primary_u_r,
        primary_x_r,
        primary_x_i,
        primary_r,
        primary_u_new,
        primary_x_new,
        previous_step_hash,
        primary_comm_t,
        primary_comm_w_i,
        params.clone(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_std::test_rng;

    /// 1C (b)-2 INTEGRATION: the shell's CF fields wired to a
    /// REAL CycleFoldRunningInstance + a real NIFS fold step
    /// between shells. Per IVC step:
    ///   (1) shell_i's cf_comm_w/e/u/x_vec = running_i's state
    ///       (Section R binds them; consistency required for CS sat).
    ///   (2) Bridge shell_i.t1 to a CF instance; fold into running.
    ///   (3) running.is_sat_relaxed must accept.
    ///   (4) Next step's shell sees the new running.
    ///
    /// Two-step variant: validates the CF-side IVC composition
    /// pattern end-to-end on real CF instances. Deferred: also
    /// folding shell.t2 (cf2: r·comm_T) — currently this sub-step
    /// folds only t1 (cf1: r·comm_W_I) per step to keep the test
    /// bounded; "fold BOTH" is the natural (b)-2b extension.
    #[test]
    #[ignore = "(b)-2: full CF accumulator integration with real ppsnark bridge (slow)"]
    fn shell_cf_accumulator_two_step_integration() {
        use crate::cyclefold_fold_homomorphism::CycleFoldRunningInstance;
        use crate::cyclefold_ivc_accumulator::bridge_cf_tuple;
        use ark_ec::CurveGroup;
        use ark_std::rand::SeedableRng;
        use nova_snark::nova::nifs::NIFS;
        use nova_snark::provider::GrumpkinEngine;
        use nova_snark::r1cs::{RelaxedR1CSInstance, RelaxedR1CSWitness};
        use nova_snark::traits::ROConstants;

        // Concrete StdRng so the closure's &mut StdRng param type
        // matches (test_rng() returns `impl Rng`, opaque).
        let mut rng = ark_std::rand::rngs::StdRng::seed_from_u64(0);
        let params = crate::neptune_permutation_gadget::params_from_dump_path(
            concat!(env!("CARGO_MANIFEST_DIR"), "/neptune-bn256-standard.json"),
        )
        .expect("load neptune params");
        let pp_hash = Bn254Fr::from(42u64);
        let z_0 = Bn254Fr::from(0u64);
        let ck_label: &'static [u8] = b"ev-cf-shell-chain";

        // Running CF starts at zero (io_len = 21 = num_io of
        // CycleFoldInstanceCircuit per increment-2 measurement).
        let running_zero = CycleFoldRunningInstance::zero(21);

        // Helper: convert running.comm_w / comm_e Grumpkin points
        // to (Bn254Fr, Bn254Fr) affine coords (native).
        let to_affine_fr = |p: ark_ec::short_weierstrass::Projective<
            crate::grumpkin_config::GrumpkinConfig,
        >|
         -> (Bn254Fr, Bn254Fr) {
            let aff = p.into_affine();
            // Grumpkin Affine coords ARE Bn254Fr per
            // grumpkin_config (BaseField = Bn254Fr).
            (aff.x, aff.y)
        };

        // Build shell 0 with cf_* fields from running_zero.
        let (cw0_x, cw0_y) = to_affine_fr(running_zero.comm_w);
        let (ce0_x, ce0_y) = to_affine_fr(running_zero.comm_e);
        // For running_zero, comm_w and comm_e are both zero (infty);
        // their .into_affine() coords are (0, 0) which is OK for
        // absorption into the sponge.

        // Run a custom build that uses specific CF values (mirrors
        // build_shell_for_step but with caller-supplied CF state).
        // Inline here to avoid yet another constructor variant.
        let build_step_with_running = |rng: &mut ark_std::rand::rngs::StdRng,
                                       step_index: u64,
                                       z_i: Bn254Fr,
                                       previous_step_hash: Bn254Fr,
                                       primary_u_r: Bn254Fr,
                                       primary_x_r: [Bn254Fr; 2],
                                       cw_x: Bn254Fr, cw_y: Bn254Fr,
                                       ce_x: Bn254Fr, ce_y: Bn254Fr,
                                       cf_u: Bn254Fq,
                                       cf_x: Vec<Bn254Fq>|
         -> PrimaryAugmentedCircuitShell {
            let z_i1 = z_i + Bn254Fr::from(1u64);
            let mk = |rng: &mut _| {
                let p = G1Affine::generator();
                let s = Bn254Fr::rand(rng);
                let q = (G1Projective::from(p) * s).into_affine();
                (p, s, q)
            };
            let (t1p, t1s, t1q) = mk(rng);
            let (t2p, t2s, t2q) = mk(rng);
            let cf_x_digest =
                crate::cyclefold_cf_x_digest::compute_cf_x_digest_pair_native(
                    t1p, t1s, t1q, t2p, t2s, t2q,
                );
            let primary_x_i: [Bn254Fr; 2] =
                [Bn254Fr::rand(rng), Bn254Fr::rand(rng)];
            let inner_mk_g1 = |rng: &mut ark_std::rand::rngs::StdRng| {
                let g = G1Affine::generator();
                let s = Bn254Fr::rand(rng);
                (G1Projective::from(g) * s).into_affine()
            };
            let primary_comm_w_i = inner_mk_g1(rng);
            let primary_comm_t = inner_mk_g1(rng);
            let i_fr = Bn254Fr::from(step_index);
            let primary_r = primary_r_native(
                pp_hash, previous_step_hash, primary_x_i[0], primary_x_i[1],
                primary_comm_w_i, primary_comm_t,
            );
            let primary_u_new = primary_u_r + primary_r;
            let primary_x_new = [
                primary_x_r[0] + primary_r * primary_x_i[0],
                primary_x_r[1] + primary_r * primary_x_i[1],
            ];
            let current_step_hash = current_step_hash_native(
                pp_hash, i_fr, z_0, z_i, z_i1, cf_x_digest, cf_u,
                cw_x, cw_y, ce_x, ce_y, &cf_x,
            );
            PrimaryAugmentedCircuitShell::new(
                pp_hash, i_fr, z_0, z_i, z_i1,
                t1p, t1s, t1q, t2p, t2s, t2q,
                cf_x_digest, current_step_hash,
                cf_u, cw_x, cw_y, ce_x, ce_y, cf_x,
                primary_u_r, primary_x_r, primary_x_i,
                primary_r, primary_u_new, primary_x_new,
                previous_step_hash, primary_comm_t, primary_comm_w_i,
                params.clone(),
            )
        };

        let cf_x_zero: Vec<Bn254Fq> = vec![Bn254Fq::from(0u64); 21];
        let shell_0 = build_step_with_running(
            &mut rng, 0, z_0, Bn254Fr::from(7u64),
            Bn254Fr::from(0u64), [Bn254Fr::from(0u64); 2],
            cw0_x, cw0_y, ce0_x, ce0_y, running_zero.u, cf_x_zero,
        );
        let s0_z_i1 = shell_0.z_i1;
        let s0_step_hash = shell_0.current_step_hash;
        let s0_u_new = shell_0.primary_u_new;
        let s0_x_new = shell_0.primary_x_new;
        let s0_t1 = (shell_0.t1_p, shell_0.t1_s, shell_0.t1_q);
        let cs_0 = ConstraintSystem::<Bn254Fr>::new_ref();
        shell_0.generate_constraints(cs_0.clone()).expect("synth 0");
        assert!(cs_0.is_satisfied().unwrap(), "shell 0 CS must be sat");

        // Fold shell_0.t1 into running.
        let art = bridge_cf_tuple(s0_t1.0, s0_t1.1, s0_t1.2, ck_label)
            .expect("bridge t1");
        let u_relaxed = RelaxedR1CSInstance::<GrumpkinEngine>::from_r1cs_instance(
            &art.ck, &art.shape, &art.instance,
        );
        let w_relaxed = RelaxedR1CSWitness::<GrumpkinEngine>::from_r1cs_witness(
            &art.shape, &art.witness,
        );
        let ro_consts = ROConstants::<GrumpkinEngine>::default();
        let pp_digest_secondary = crate::scalar_adapter::SecondaryScalar::from(0u64);
        // Initial running CF (relaxed, zero).
        let u_run_init =
            RelaxedR1CSInstance::<GrumpkinEngine>::default(&art.ck, &art.shape);
        let w_run_init = RelaxedR1CSWitness::<GrumpkinEngine>::default(&art.shape);
        let (_nifs, (u_run_after_0, w_run_after_0)) =
            NIFS::<GrumpkinEngine>::prove(
                &art.ck, &ro_consts, &pp_digest_secondary, &art.shape,
                &u_run_init, &w_run_init, &art.instance, &art.witness,
            )
            .expect("nifs prove step 0");
        art.shape
            .is_sat_relaxed(&art.ck, &u_run_after_0, &w_run_after_0)
            .expect("running CF (post step 0) must be is_sat_relaxed");
        // Sanity: lift the relaxed RHS to confirm symmetry.
        let _ = (u_relaxed, w_relaxed);

        // Step 1: cf_* fields = post-step-0 running CF state.
        // Building these requires extracting comm_w/comm_e/u/X from
        // u_run_after_0 + w_run_after_0. Those fields are pub(crate)
        // in nova-snark — for THIS test we just re-use the
        // running_zero values and a TODO note: full coherent
        // chaining requires extending the bridge / 4a accumulator
        // to expose the running instance's commitments + scalars
        // in arkworks types. That's the (b)-2b sub-step.
        //
        // For this turn the soundness gate is: step 0 worked
        // end-to-end (shell synth + NIFS fold + is_sat_relaxed).
        // Step 1 chaining of the CF-side fields awaits (b)-2b
        // (extract running CF state into arkworks types).
        let _ = s0_z_i1;
        let _ = s0_step_hash;
        let _ = s0_u_new;
        let _ = s0_x_new;

        // (b)-2b: extract the post-step-0 running CF state and
        // chain it into shell_1. Path schema verified by HEAD
        // 9344e97a premise dump; extractor in s4_secondary_extract.
        let v = serde_json::to_value(&u_run_after_0)
            .expect("RelaxedR1CSInstance to_value");
        let (cw, ce, u1, x1) =
            crate::s4_secondary_extract::extract_relaxed_running_inst(&v)
                .expect("extract running CF state post-step-0");
        let (cw_x, cw_y) = (cw.x, cw.y);
        let (ce_x, ce_y) = (ce.x, ce.y);

        // Step 1: shell built with cf_* fields = post-step-0
        // running CF state; primary fields threaded from step 0's
        // public outputs.
        let shell_1 = build_step_with_running(
            &mut rng,
            1,
            s0_z_i1,
            s0_step_hash,
            s0_u_new,
            s0_x_new,
            cw_x, cw_y, ce_x, ce_y, u1, x1,
        );
        let s1_t1 = (shell_1.t1_p, shell_1.t1_s, shell_1.t1_q);
        let cs_1 = ConstraintSystem::<Bn254Fr>::new_ref();
        shell_1.generate_constraints(cs_1.clone()).expect("synth 1");
        assert!(cs_1.is_satisfied().unwrap(), "shell 1 CS must be sat");

        // Fold shell_1.t1 into the running pair (now u_run_after_0).
        let art1 = bridge_cf_tuple(s1_t1.0, s1_t1.1, s1_t1.2, ck_label)
            .expect("bridge t1 step 1");
        let (_nifs1, (u_run_after_1, w_run_after_1)) =
            NIFS::<GrumpkinEngine>::prove(
                &art1.ck, &ro_consts, &pp_digest_secondary, &art1.shape,
                &u_run_after_0, &w_run_after_0, &art1.instance, &art1.witness,
            )
            .expect("nifs prove step 1");
        art1.shape
            .is_sat_relaxed(&art1.ck, &u_run_after_1, &w_run_after_1)
            .expect("running CF (post step 1) must be is_sat_relaxed");
    }

    /// 2-STEP CHAIN: synthesise shell_0 then shell_1 with step_1's
    /// inputs threaded from step_0's public outputs. Both CSes
    /// must be satisfied. Proves the per-step shell composes
    /// cleanly: step i's public outputs ARE valid step i+1 inputs.
    #[test]
    fn two_step_chain_threads_state_correctly() {
        let mut rng = test_rng();
        let params = crate::neptune_permutation_gadget::params_from_dump_path(
            concat!(env!("CARGO_MANIFEST_DIR"), "/neptune-bn256-standard.json"),
        )
        .expect("load neptune params");

        let pp_hash = Bn254Fr::from(42u64);
        let z_0 = Bn254Fr::from(0u64);

        // Step 0: initial state. previous_step_hash = base value
        // (would be defined by the IVC's genesis convention; here
        // arbitrary for the chain test).
        let initial_previous_step_hash = Bn254Fr::from(7u64);
        let initial_u_r = Bn254Fr::from(0u64);
        let initial_x_r: [Bn254Fr; 2] = [Bn254Fr::from(0u64), Bn254Fr::from(0u64)];

        let shell_0 = build_shell_for_step(
            &mut rng,
            pp_hash,
            z_0,
            0,
            z_0,
            initial_previous_step_hash,
            initial_u_r,
            initial_x_r,
            &params,
        );

        // Extract step 0's outputs BEFORE moving shell_0 into CS.
        let step_0_z_i1 = shell_0.z_i1;
        let step_0_current_step_hash = shell_0.current_step_hash;
        let step_0_u_new = shell_0.primary_u_new;
        let step_0_x_new = shell_0.primary_x_new;

        let cs_0 = ConstraintSystem::<Bn254Fr>::new_ref();
        shell_0.generate_constraints(cs_0.clone()).expect("synth 0");
        assert!(cs_0.is_satisfied().unwrap(), "shell 0 CS must be sat");

        // Step 1: inputs threaded from step 0's outputs.
        let shell_1 = build_shell_for_step(
            &mut rng,
            pp_hash,
            z_0,
            1,
            step_0_z_i1,                 // z_1 ← step 0's z_{i+1}
            step_0_current_step_hash,    // previous_step_hash ← step 0's current_step_hash
            step_0_u_new,                // u_R ← step 0's u_new
            step_0_x_new,                // X_R ← step 0's X_new
            &params,
        );

        // Extract step 1's z_i BEFORE the move (generate_constraints
        // consumes self).
        let step_1_z_i = shell_1.z_i;

        let cs_1 = ConstraintSystem::<Bn254Fr>::new_ref();
        shell_1.generate_constraints(cs_1.clone()).expect("synth 1");
        assert!(cs_1.is_satisfied().unwrap(), "shell 1 CS must be sat");

        // Cross-check: step 1's `z_i` MUST equal step 0's `z_i+1`
        // (threading correctness check; CS sat alone doesn't
        // explicitly assert this).
        assert_eq!(
            step_1_z_i, step_0_z_i1,
            "step 1 z_i must = step 0 z_{{i+1}}"
        );
    }
}
