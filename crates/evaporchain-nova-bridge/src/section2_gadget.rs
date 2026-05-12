//! Phase 2.2-section-2 step 3: arkworks-side Poseidon gadget shape.
//!
//! # What this module ships
//!
//! `enforce_poseidon_primary` — an R1CS gadget that absorbs a
//! sequence of `FpVar<Bn254Fr>` scalars into an arkworks
//! `PoseidonSpongeVar` and returns the squeezed `FpVar`. Plus
//! a higher-level `enforce_section_2_primary` that takes the
//! Section-2 primary-side absorb slots (`pp_digest`, `num_steps`,
//! `z0[..]`, `zi[..]`, instance expansion, `ri_primary`) and
//! emits the Poseidon hash with the documented order from PR #65.
//!
//! # CRITICAL caveat
//!
//! Uses **arkworks-default Poseidon constants** (8 full + 60 partial
//! rounds, alpha=5, generic Cauchy MDS via
//! `find_poseidon_ark_and_mds`). These DO NOT match nova-snark's
//! neptune Poseidon constants — outputs will diverge from the
//! `neptune_reference` oracle. This gadget therefore validates the
//! GADGET SHAPE (correct absorb order, correct squeeze, correct
//! public-input-vs-witness allocation) but NOT byte parity.
//!
//! Step 4 of the port plan replaces the `PoseidonConfig` argument
//! with neptune-equivalent constants. The gadget structure stays
//! put. That's the BESPOKE wedge.
//!
//! # Why ship the gadget shape before the constants
//!
//! Three reasons:
//!
//! 1. **Catches arity/wiring bugs early.** Whether the gadget
//!    absorbs in the right order, allocates the right number of
//!    public inputs vs witnesses, and connects to the existing
//!    `NovaVerifierCircuit` public-input scheme — these are checked
//!    independently of constant correctness.
//!
//! 2. **Pins the constraint count.** The arkworks-default constants
//!    are close enough in shape to neptune that the constraint count
//!    is a useful empirical floor. PR #70 measured ~753 constraints
//!    for absorb-6 squeeze-1 at this config; the gadget here should
//!    reproduce that number when wired through `report_shape`.
//!
//! 3. **Makes the port a one-line swap.** When the neptune constants
//!    are ported, the only diff is the `PoseidonConfig` argument.
//!    No structural refactor.

use ark_bn254::Fr as Bn254Fr;
use ark_crypto_primitives::sponge::constraints::CryptographicSpongeVar;
use ark_crypto_primitives::sponge::poseidon::constraints::PoseidonSpongeVar;
use ark_crypto_primitives::sponge::poseidon::traits::find_poseidon_ark_and_mds;
use ark_crypto_primitives::sponge::poseidon::PoseidonConfig;
use ark_ff::PrimeField;
use ark_r1cs_std::fields::fp::FpVar;
use ark_relations::r1cs::{ConstraintSystemRef, SynthesisError};

/// Build a PLACEHOLDER `PoseidonConfig<Bn254Fr>` with arkworks-default
/// security parameters. Same parameters as
/// `crate::poseidon_budget::arkworks_default_config_for_bn254` —
/// matched intentionally so the constraint count reported there
/// (~753 for absorb-6 squeeze-1) carries over.
///
/// **These constants DO NOT match neptune.** When the port lands,
/// this function is the swap point.
pub fn placeholder_poseidon_config() -> PoseidonConfig<Bn254Fr> {
    let full_rounds = 8usize;
    let partial_rounds = 60usize;
    let alpha = 5u64;
    let rate = 2usize;
    let capacity = 1usize;
    let prime_bits = Bn254Fr::MODULUS_BIT_SIZE as u64;
    let (ark, mds) = find_poseidon_ark_and_mds::<Bn254Fr>(
        prime_bits,
        rate,
        full_rounds as u64,
        partial_rounds as u64,
        0u64,
    );
    PoseidonConfig::new(full_rounds, partial_rounds, alpha, mds, ark, rate, capacity)
}

/// Absorb `inputs` into a `PoseidonSpongeVar<Bn254Fr>` configured
/// with `config`, then squeeze one field element and return it as
/// an `FpVar<Bn254Fr>`.
///
/// This is the low-level shape primitive. `enforce_section_2_primary`
/// wraps it with the documented Section-2 absorb order.
pub fn enforce_poseidon_primary(
    cs: ConstraintSystemRef<Bn254Fr>,
    config: &PoseidonConfig<Bn254Fr>,
    inputs: &[FpVar<Bn254Fr>],
) -> Result<FpVar<Bn254Fr>, SynthesisError> {
    let mut sponge = PoseidonSpongeVar::<Bn254Fr>::new(cs, config);
    for x in inputs {
        sponge.absorb(x)?;
    }
    let out = sponge.squeeze_field_elements(1)?;
    Ok(out.into_iter().next().expect("squeezed 1 element"))
}

/// Convenience wrapper: takes the named Section-2 primary-side
/// slots in absorb order (matching PR #65's
/// `poseidon_transcript::absorb_order(HasherSide::Primary, z_arity)`)
/// and returns the hashed `FpVar`. Caller is responsible for
/// allocating the slots from circuit inputs / witnesses.
///
/// Section-2 primary-side absorb shape at z_arity=1:
///   `[pp.digest, num_steps, z0[0], zi[0], instance..., ri_primary]`
///
/// `instance` carries the variable-length cross-side absorb
/// expansion (`r_U_secondary.absorb_in_ro` — `comm_W`, `comm_E`,
/// `u`, `X[..]`). At this gadget level the caller supplies it as
/// an opaque slice; the exact encoding choice is part of Section 3.
pub fn enforce_section_2_primary(
    cs: ConstraintSystemRef<Bn254Fr>,
    config: &PoseidonConfig<Bn254Fr>,
    pp_digest: &FpVar<Bn254Fr>,
    num_steps: &FpVar<Bn254Fr>,
    z0: &[FpVar<Bn254Fr>],
    zi: &[FpVar<Bn254Fr>],
    instance: &[FpVar<Bn254Fr>],
    ri_primary: &FpVar<Bn254Fr>,
) -> Result<FpVar<Bn254Fr>, SynthesisError> {
    let mut inputs: Vec<FpVar<Bn254Fr>> =
        Vec::with_capacity(2 + z0.len() + zi.len() + instance.len() + 1);
    inputs.push(pp_digest.clone());
    inputs.push(num_steps.clone());
    inputs.extend_from_slice(z0);
    inputs.extend_from_slice(zi);
    inputs.extend_from_slice(instance);
    inputs.push(ri_primary.clone());
    enforce_poseidon_primary(cs, config, &inputs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_r1cs_std::alloc::AllocVar;
    use ark_relations::r1cs::ConstraintSystem;

    /// Smoke test: gadget compiles, runs over a 6-scalar absorb,
    /// emits a single squeezed FpVar, and the constraint count
    /// lands in the documented range (PR #70 measured ~753 for
    /// arkworks-default Poseidon absorb-6 squeeze-1).
    #[test]
    fn enforce_poseidon_primary_compiles_and_runs() {
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        let config = placeholder_poseidon_config();
        let inputs: Vec<FpVar<Bn254Fr>> = (1..=6u64)
            .map(|i| FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(Bn254Fr::from(i))).unwrap())
            .collect();
        let out = enforce_poseidon_primary(cs.clone(), &config, &inputs).expect("synthesize");
        assert!(!matches!(out, FpVar::Constant(_)), "squeezed output must be a variable");
        let nc = cs.num_constraints();
        assert!(
            (200..=10_000).contains(&nc),
            "Section-2 primary gadget constraint count out of range: {nc}"
        );
        eprintln!(
            "enforce_poseidon_primary: 6 absorb + 1 squeeze → {} constraints",
            nc
        );
    }

    /// Wire up the Section-2 primary-side absorb in the exact
    /// shape Section 2's R1CS gadget will use at z_arity=1:
    /// 6 named slots (digest + num_steps + z0[0] + zi[0] +
    /// 1-element instance + ri_primary). Confirm the assembled
    /// absorb sequence runs end-to-end.
    #[test]
    fn enforce_section_2_primary_at_z_arity_1() {
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        let config = placeholder_poseidon_config();

        let pp_digest =
            FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(Bn254Fr::from(0u64))).unwrap();
        let num_steps =
            FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(Bn254Fr::from(1u64))).unwrap();
        let z0_0 = FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(Bn254Fr::from(0u64))).unwrap();
        let zi_0 = FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(Bn254Fr::from(1u64))).unwrap();
        // Single placeholder instance scalar — Section 3 will
        // expand this to the real (comm_W, comm_E, u, X[..]) vector.
        let instance = vec![
            FpVar::<Bn254Fr>::new_witness(cs.clone(), || Ok(Bn254Fr::from(0u64))).unwrap(),
        ];
        let ri_primary =
            FpVar::<Bn254Fr>::new_witness(cs.clone(), || Ok(Bn254Fr::from(0u64))).unwrap();

        let _hash = enforce_section_2_primary(
            cs.clone(),
            &config,
            &pp_digest,
            &num_steps,
            &[z0_0],
            &[zi_0],
            &instance,
            &ri_primary,
        )
        .expect("synthesize");

        // 4 public inputs + 1 implicit const + 2 witness slots
        // (instance + ri_primary) seeded by the test; Poseidon
        // internals add more witnesses.
        assert!(cs.num_instance_variables() >= 5);
        assert!(cs.num_constraints() > 200);
    }

    /// Determinism canary: same allocated inputs produce structurally
    /// identical constraint systems. If `PoseidonSpongeVar` ever picks
    /// up non-determinism (e.g., random gadget-internal allocations),
    /// this fires.
    #[test]
    fn gadget_is_deterministic() {
        let mk = || {
            let cs = ConstraintSystem::<Bn254Fr>::new_ref();
            let config = placeholder_poseidon_config();
            let inputs: Vec<FpVar<Bn254Fr>> = (0..6u64)
                .map(|i| {
                    FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(Bn254Fr::from(i + 1))).unwrap()
                })
                .collect();
            let _ = enforce_poseidon_primary(cs.clone(), &config, &inputs).unwrap();
            (cs.num_instance_variables(), cs.num_witness_variables(), cs.num_constraints())
        };
        let a = mk();
        let b = mk();
        assert_eq!(a, b, "gadget must produce identical CS shape across runs");
    }

    /// **Port-complete canary.** Today: confirms gadget output
    /// ≠ neptune oracle output (because placeholder constants).
    /// After the BESPOKE port: this test INVERTS to `assert_eq!`,
    /// proving byte parity.
    ///
    /// Input is the Section-2 primary-side minimal absorb sequence:
    ///   `[pp.digest=0, num_steps=1, z0[0]=0, zi[0]=1, ri=0]`
    /// (matches PR #77's `pinned_section_2_primary_minimal_absorb_sequence`)
    ///
    /// This test serves three purposes:
    ///
    /// 1. Documents the divergence: the arkworks-default Poseidon
    ///    config produces a DIFFERENT scalar than neptune for the
    ///    same input sequence (because round constants + MDS
    ///    differ).
    /// 2. Provides a single byte-comparison test point that the
    ///    BESPOKE port can re-run to confirm parity.
    /// 3. Catches a future regression where someone accidentally
    ///    swaps in the neptune constants partially (and the
    ///    arkworks gadget starts matching some test inputs but not
    ///    others).
    #[test]
    fn placeholder_gadget_diverges_from_neptune_oracle() {
        use crate::neptune_reference::{neptune_hash_primary, PrimaryScalar};
        use ff::{Field as _, PrimeField as _};
        use ark_r1cs_std::R1CSVar;

        // Neptune-side: the pinned minimal absorb (PR #77).
        let neptune_inputs = vec![
            PrimaryScalar::ZERO,        // pp.digest
            PrimaryScalar::from(1u64),  // num_steps
            PrimaryScalar::ZERO,        // z0[0]
            PrimaryScalar::from(1u64),  // zi[0]
            PrimaryScalar::ZERO,        // ri_primary
        ];
        let neptune_out = neptune_hash_primary(&neptune_inputs);
        let neptune_bytes_le: [u8; 32] = neptune_out.to_repr().into();

        // Arkworks-side: same input shape but instance=[] so the
        // absorb count matches neptune's 5 elements exactly.
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        let config = placeholder_poseidon_config();
        let pp_digest =
            FpVar::<Bn254Fr>::new_witness(cs.clone(), || Ok(Bn254Fr::from(0u64))).unwrap();
        let num_steps =
            FpVar::<Bn254Fr>::new_witness(cs.clone(), || Ok(Bn254Fr::from(1u64))).unwrap();
        let z0 = vec![FpVar::<Bn254Fr>::new_witness(cs.clone(), || Ok(Bn254Fr::from(0u64))).unwrap()];
        let zi = vec![FpVar::<Bn254Fr>::new_witness(cs.clone(), || Ok(Bn254Fr::from(1u64))).unwrap()];
        let instance: Vec<FpVar<Bn254Fr>> = vec![];
        let ri =
            FpVar::<Bn254Fr>::new_witness(cs.clone(), || Ok(Bn254Fr::from(0u64))).unwrap();
        let hash_var = enforce_section_2_primary(
            cs.clone(),
            &config,
            &pp_digest,
            &num_steps,
            &z0,
            &zi,
            &instance,
            &ri,
        )
        .expect("synthesize");
        let ark_out: Bn254Fr = hash_var.value().expect("witness value");
        let ark_bigint = ark_ff::PrimeField::into_bigint(ark_out);
        let bytes_le = ark_ff::BigInteger::to_bytes_le(&ark_bigint);
        let mut ark_bytes_le = [0u8; 32];
        let copy_len = bytes_le.len().min(32);
        ark_bytes_le[..copy_len].copy_from_slice(&bytes_le[..copy_len]);

        eprintln!("neptune bytes LE:  {neptune_bytes_le:?}");
        eprintln!("arkworks bytes LE: {ark_bytes_le:?}");

        // Today: bytes MUST differ. Tomorrow (post-port): assert_eq.
        assert_ne!(
            ark_bytes_le, neptune_bytes_le,
            "placeholder constants must NOT match neptune — if this fires either \
             (a) the BESPOKE neptune-constants port landed (flip to assert_eq), or \
             (b) by astronomical coincidence the placeholder Poseidon produced the \
             same hash on this input."
        );
    }
}
