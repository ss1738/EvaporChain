//! Phase 2.4 starter — Groth16 trusted setup over
//! [`crate::verifier_circuit::NovaVerifierCircuit`].
//!
//! # What ships at this milestone
//!
//! [`setup`] — wraps `Groth16::<Bn254>::circuit_specific_setup`
//! over [`NovaVerifierCircuit::setup_shape`] (audit B-1/B-2 S2a):
//! the canonical *section-bearing* circuit, producing a
//! `(ProvingKey, VerifyingKey)` pair (5 public inputs: 2 hashes +
//! 1 z0 + 1 zi + 1 Groth16 const).
//!
//! # Audit B-1/B-2: why the keyed circuit is sound (S2a + S2b)
//!
//! The original B-1 hazard was keying setup over `dummy()` — a
//! constraint-vacuous, section-less circuit — on the *false*
//! assumption that its R1CS shape equals a real prover's. It does
//! not: a real prover circuit carries the Section 2 (Neptune
//! transcript) and Section 3 (primary RelaxedR1CS-sat) binding
//! constraints; `dummy()` carries none, so Groth16 keys built over
//! it are forgeable.
//!
//! S2a fixes this: setup is keyed over `setup_shape()`, whose R1CS
//! is proven *bit-identical* to a real-witness prover circuit by
//! the S6 determinism test
//! (`circuit_builder::tests::s2a_setup_shape_matches_real_prover_r1cs`).
//! S2b makes the section bindings MANDATORY in
//! `generate_constraints` (and `validate_structurally` rejects a
//! section-less witness), so a prover cannot substitute the
//! vacuous circuit the keys were *not* built for.
//!
//! # Trusted-setup caveat (production deployment) — S5, still open
//!
//! `circuit_specific_setup` uses arkworks's *insecure* test
//! randomness. For mainnet, the keys must come from a multi-party
//! ceremony (e.g. Powers of Tau + circuit-specific phase 2). This
//! module's `setup` is the right entry point for testing and for
//! development, but is **not** production-safe. Production swap-in
//! point is the same function signature with the ceremony-derived
//! parameters substituted.

use ark_bn254::{Bn254, Fr as Bn254Fr};
use ark_groth16::{Groth16, Proof, ProvingKey, VerifyingKey};
use ark_snark::SNARK;
use ark_std::rand::{CryptoRng, RngCore};

use crate::verifier_circuit::NovaVerifierCircuit;

/// Run Groth16's circuit-specific trusted setup over
/// [`NovaVerifierCircuit::setup_shape`] (the canonical
/// section-bearing circuit). Returns `(pk, vk)`.
///
/// **Test/dev only.** See module docstring for the production
/// trusted-setup story.
///
/// # SECURITY (audit B-1/B-2)
///
/// B-1 (constraint-vacuous keyed circuit) is CLOSED: setup is keyed
/// over `setup_shape()`, whose R1CS is proven bit-identical to a
/// real prover's by the S6 determinism test, and S2b makes the
/// Section 2/3 bindings mandatory (non-`Option`). B-2 is NOT closed:
/// `circuit_specific_setup` still uses arkworks's *insecure* test
/// randomness (the "toxic waste" is recoverable), so keys from this
/// function remain forgeable and MUST NOT reach mainnet. Production
/// requires an MPC ceremony (Powers of Tau + circuit-specific phase
/// 2) — tracked as the S5 sub-stage. This `#[deprecated]` marker makes
/// every call site emit a build warning so the insecure path cannot
/// be shipped silently; it is intentionally not removed until the
/// ceremony-derived, sound-circuit replacement lands.
#[deprecated(note = "INSECURE test/dev trusted setup (audit B-1/B-2): recoverable \
            toxic waste + a constraint-vacuous circuit — forgeable keys. \
            MUST NOT reach mainnet. Replace with the fixed-shape \
            section-bearing circuit + MPC ceremony (audit #1 \
            mainnet-blocker) before any production use.")]
pub fn setup<R: RngCore + CryptoRng>(
    rng: &mut R,
) -> Result<(ProvingKey<Bn254>, VerifyingKey<Bn254>), ark_relations::gr1cs::SynthesisError> {
    // Audit B-1/B-2 S2a: key the circuit over the section-bearing
    // setup_shape() (sections at exact prover R1CS shape) — NOT the
    // constraint-vacuous dummy() — so the soundness bindings are part
    // of the keyed circuit. The `#[deprecated]` marker above still
    // applies: B-1 vacuity is closed once S2b lands (mandatory
    // generate_constraints emission); B-2 toxic-waste closes only
    // with the S5 MPC ceremony.
    let circuit = NovaVerifierCircuit::setup_shape()
        .map_err(|_| ark_relations::gr1cs::SynthesisError::Unsatisfiable)?;
    Groth16::<Bn254>::circuit_specific_setup(circuit, rng)
}

/// Generate a Groth16 proof for a concrete `NovaVerifierCircuit`
/// witness, using the provided proving key.
///
/// The circuit can be any `NovaVerifierCircuit` that synthesises
/// a satisfied CS — `NovaVerifierCircuit::dummy()` works today;
/// `circuit_builder::build_circuit_from_fixture(&rs)` will work
/// once Sections 2/3 wire up.
pub fn prove<R: RngCore + CryptoRng>(
    pk: &ProvingKey<Bn254>,
    circuit: NovaVerifierCircuit,
    rng: &mut R,
) -> Result<Proof<Bn254>, ark_relations::gr1cs::SynthesisError> {
    Groth16::<Bn254>::prove(pk, circuit, rng)
}

/// Verify a Groth16 proof against the verifying key + public
/// inputs.
///
/// `public_inputs` must be supplied in the same order as the
/// circuit allocates them via `new_input`:
///
///   `[committed_hash_primary, committed_hash_secondary, z0[..], zi[..]]`
///
/// (The Groth16 const-1 input is added implicitly by arkworks
/// and is not part of this slice.)
pub fn verify(
    vk: &VerifyingKey<Bn254>,
    public_inputs: &[Bn254Fr],
    proof: &Proof<Bn254>,
) -> Result<bool, ark_relations::gr1cs::SynthesisError> {
    Groth16::<Bn254>::verify(vk, public_inputs, proof)
}

/// Convenience: derive the public-inputs slice for a given
/// `NovaVerifierCircuit` in the order [`verify`] expects.
///
/// Useful when the same `NovaVerifierCircuit` is fed to [`prove`]
/// — both sides must agree on the slice, and writing the order
/// out twice at call sites invites drift.
pub fn public_inputs_for(circuit: &NovaVerifierCircuit) -> Vec<Bn254Fr> {
    let mut out = Vec::with_capacity(2 + circuit.z0.len() + circuit.zi.len());
    out.push(circuit.committed_hash_primary);
    out.push(circuit.committed_hash_secondary);
    out.extend_from_slice(&circuit.z0);
    out.extend_from_slice(&circuit.zi);
    out
}

// ─── B-1/B-2 1C §7 step 1: Groth16 plumbing for RecursionDeciderCircuit ───
//
// The old `setup`/`prove`/`verify` above key Groth16 over the dead
// `NovaVerifierCircuit` (S4b 203M-cons path). The functions below key
// Groth16 over the LIVE `RecursionDeciderCircuit` (Section A wired,
// ~26.7M cons at n=10,554; ~43.5M at n=16,384 per (d)-1 measurement).
// These coexist with the old functions during the transition — the
// old ones stay until the full B/C/D wiring lands and the dossier
// can move the NovaVerifierCircuit path to deletion.

use crate::grumpkin_config::GrumpkinConfig;
use crate::recursion_decider_circuit::RecursionDeciderCircuit;
use ark_ec::short_weierstrass::Affine;

/// Groth16 trusted setup keyed over
/// `RecursionDeciderCircuit::setup_shape(bases, h)`. The bases vector
/// is BAKED INTO THE CIRCUIT shape — setup and prove MUST use the
/// exact same `bases` (and `h`).
///
/// At n=4–8 this is fast (smoke-test scale). At the real n_aux=16,384
/// it is heavy (hours on the Mini cluster; ~5.6 GB peak memory per
/// the (d)-1 cons budget).
///
/// **Test/dev only** — the same `#[deprecated]` MPC-ceremony caveat
/// from `setup()` above applies; this function uses arkworks's
/// insecure test randomness.
#[deprecated(note = "INSECURE test/dev trusted setup (audit B-1/B-2 S5): \
            recoverable toxic waste. MUST NOT reach mainnet. The \
            circuit shape (RecursionDeciderCircuit) is the correct \
            B-1/B-2 1C path, but production keys require an MPC \
            ceremony (Powers of Tau + circuit-specific phase 2).")]
pub fn setup_recursion_decider<R: RngCore + CryptoRng>(
    bases: Vec<Affine<GrumpkinConfig>>,
    h: Affine<GrumpkinConfig>,
    rng: &mut R,
) -> Result<(ProvingKey<Bn254>, VerifyingKey<Bn254>), ark_relations::gr1cs::SynthesisError> {
    let circuit = RecursionDeciderCircuit::setup_shape(bases, h);
    Groth16::<Bn254>::circuit_specific_setup(circuit, rng)
}

/// Prove a `RecursionDeciderCircuit` witness with `pk` from
/// `setup_recursion_decider`. The `circuit`'s `bases` + `h` must
/// match the bases + h that produced `pk` exactly.
pub fn prove_recursion_decider<R: RngCore + CryptoRng>(
    pk: &ProvingKey<Bn254>,
    circuit: RecursionDeciderCircuit,
    rng: &mut R,
) -> Result<Proof<Bn254>, ark_relations::gr1cs::SynthesisError> {
    Groth16::<Bn254>::prove(pk, circuit, rng)
}

/// Verify a `RecursionDeciderCircuit` Groth16 proof. The Section A
/// circuit currently allocates ZERO `new_input` calls — Sections
/// B/C/D will add the (committed_hash_primary, committed_hash_secondary,
/// z0, zi) public-input layout when wired. For now the public-inputs
/// slice is empty.
pub fn verify_recursion_decider(
    vk: &VerifyingKey<Bn254>,
    public_inputs: &[Bn254Fr],
    proof: &Proof<Bn254>,
) -> Result<bool, ark_relations::gr1cs::SynthesisError> {
    Groth16::<Bn254>::verify(vk, public_inputs, proof)
}

/// Section-B-aware Groth16 trusted setup. Keys the circuit over
/// `RecursionDeciderCircuit::setup_shape_with_b_interface`, which
/// allocates both Section A's MSM constraints AND Section B's
/// 9+|z0|+|zn| public-input slots. The prover at prove-time must
/// supply a circuit constructed via
/// `section_a_with_b_interface(...)` with matching `pi_arity` so
/// the PI count aligns with the keyed shape.
///
/// Per dossier §6b: Section B is in-circuit allocation only (no
/// enforcement); the binding lives in the off-chain
/// `assemble_section_b_pi_bundle` adapter.
#[deprecated(note = "INSECURE test/dev trusted setup. Same MPC caveat as setup_recursion_decider.")]
pub fn setup_recursion_decider_with_b_interface<R: RngCore + CryptoRng>(
    bases: Vec<Affine<GrumpkinConfig>>,
    h: Affine<GrumpkinConfig>,
    pi_arity: usize,
    rng: &mut R,
) -> Result<(ProvingKey<Bn254>, VerifyingKey<Bn254>), ark_relations::gr1cs::SynthesisError> {
    let circuit = RecursionDeciderCircuit::setup_shape_with_b_interface(bases, h, pi_arity);
    Groth16::<Bn254>::circuit_specific_setup(circuit, rng)
}

/// Helper: pack a Section B public-input bundle into the `Bn254Fr`
/// slice that `verify_recursion_decider` consumes. Order matches
/// the `new_input` calls in `RecursionDeciderCircuit::generate_constraints`:
///   hash_secondary_claimed, hash_primary_reinterp, pp_digest,
///   num_steps, ri_secondary, r_U_primary_comm_x, r_U_primary_comm_y,
///   r_U_primary_x0, r_U_primary_x1, z0[..], zn[..].
pub fn section_b_public_inputs_slice(
    b: &crate::recursion_decider_circuit::SectionBPublicInputs,
) -> Vec<Bn254Fr> {
    let mut out = Vec::with_capacity(b.pi_count());
    out.push(b.hash_secondary_claimed);
    out.push(b.hash_primary_reinterp);
    out.push(b.pp_digest);
    out.push(b.num_steps);
    out.push(b.ri_secondary);
    out.push(b.r_U_primary_comm_x);
    out.push(b.r_U_primary_comm_y);
    out.push(b.r_U_primary_x0);
    out.push(b.r_U_primary_x1);
    out.extend_from_slice(&b.z0);
    out.extend_from_slice(&b.zn);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::circuit_builder::real_provable_circuit;
    use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
    use ark_std::rand::SeedableRng;

    /// Setup produces a non-trivial (pk, vk) pair on the dummy
    /// circuit. Pins the public-input-count contract that Section
    /// 1 already enforces (5 entries) — Groth16's verification
    /// key's `gamma_abc_g1` array has length `num_instance_variables`,
    /// so we get a structural cross-check here too.
    #[test]
    fn setup_produces_keys_with_expected_public_input_count() {
        let mut rng = ark_std::rand::rngs::StdRng::seed_from_u64(0);
        let (pk, vk) = setup(&mut rng).expect("setup must succeed on setup_shape");

        // 5 public inputs as pinned by
        // `verifier_circuit::tests::skeleton_dummy_synthesizes_with_expected_public_input_arity`.
        // Groth16's verifying key adds one extra entry (the
        // constant 1 input), so `gamma_abc_g1` has length 5.
        assert_eq!(
            vk.gamma_abc_g1.len(),
            5,
            "vk.gamma_abc_g1 must hold 5 entries (2 hashes + |z0| + |zi| + 1 const)"
        );

        // Prover key cross-link: `pk.vk` must equal the returned vk.
        let mut vk_bytes = Vec::new();
        let mut pk_vk_bytes = Vec::new();
        vk.serialize_compressed(&mut vk_bytes)
            .expect("vk serialize");
        pk.vk
            .serialize_compressed(&mut pk_vk_bytes)
            .expect("pk.vk serialize");
        assert_eq!(
            vk_bytes, pk_vk_bytes,
            "pk.vk must be byte-identical to the standalone vk"
        );
    }

    /// Both keys round-trip through arkworks compressed
    /// serialization. Catches accidental dependency-bump-induced
    /// schema drift if a future arkworks version changes the
    /// canonical form.
    #[test]
    fn setup_keys_round_trip_through_canonical_serialize() {
        let mut rng = ark_std::rand::rngs::StdRng::seed_from_u64(7);
        let (pk, vk) = setup(&mut rng).expect("setup");

        let mut pk_buf = Vec::new();
        pk.serialize_compressed(&mut pk_buf).expect("pk serialize");
        let pk_back: ProvingKey<Bn254> =
            ProvingKey::deserialize_compressed(&pk_buf[..]).expect("pk deserialize");
        let mut pk_back_buf = Vec::new();
        pk_back
            .serialize_compressed(&mut pk_back_buf)
            .expect("pk_back serialize");
        assert_eq!(pk_buf, pk_back_buf, "pk round-trip must be byte-stable");

        let mut vk_buf = Vec::new();
        vk.serialize_compressed(&mut vk_buf).expect("vk serialize");
        let vk_back: VerifyingKey<Bn254> =
            VerifyingKey::deserialize_compressed(&vk_buf[..]).expect("vk deserialize");
        let mut vk_back_buf = Vec::new();
        vk_back
            .serialize_compressed(&mut vk_back_buf)
            .expect("vk_back serialize");
        assert_eq!(vk_buf, vk_back_buf, "vk round-trip must be byte-stable");
    }

    /// Setup is deterministic for a fixed seed. Catches a future
    /// arkworks bug where randomness leaks in from outside the
    /// RNG (e.g. system time, hardware-RNG fallback).
    #[test]
    fn setup_is_deterministic_for_fixed_seed() {
        let mut rng_a = ark_std::rand::rngs::StdRng::seed_from_u64(42);
        let mut rng_b = ark_std::rand::rngs::StdRng::seed_from_u64(42);
        let (pk_a, vk_a) = setup(&mut rng_a).expect("setup a");
        let (pk_b, vk_b) = setup(&mut rng_b).expect("setup b");

        let mut a_bytes = Vec::new();
        let mut b_bytes = Vec::new();
        vk_a.serialize_compressed(&mut a_bytes).unwrap();
        vk_b.serialize_compressed(&mut b_bytes).unwrap();
        assert_eq!(a_bytes, b_bytes, "vk must be deterministic for fixed seed");

        let mut pa = Vec::new();
        let mut pb = Vec::new();
        pk_a.serialize_compressed(&mut pa).unwrap();
        pk_b.serialize_compressed(&mut pb).unwrap();
        assert_eq!(pa, pb, "pk must be deterministic for fixed seed");
    }

    /// End-to-end Groth16 round-trip on the dummy circuit:
    /// setup → prove → verify(true). Pins that all three wrappers
    /// agree on the circuit shape.
    ///
    /// Audit B-1/B-2 S2b-prover: REAL end-to-end positive — a proof
    /// of a real-fixture circuit (both sections, satisfiable witness)
    /// verifies against the `setup()`-keyed pk/vk. This works
    /// precisely because S6 proves the real circuit's R1CS is
    /// bit-identical to `setup_shape()`'s. (`setup_shape()` itself is
    /// a SHAPE template with a zeroed, non-satisfiable witness — it
    /// keys setup but cannot produce a verifying proof; only a real
    /// fixture can.)
    #[test]
    #[ignore = "S2b-prover: real Nova fixture + /tmp/neptune-bn256-standard.json (expensive)"]
    fn prove_and_verify_real_fixture_round_trip_accepts() {
        let Some(circuit) = real_provable_circuit() else {
            return;
        };
        let mut rng = ark_std::rand::rngs::StdRng::seed_from_u64(101);
        let (pk, vk) = setup(&mut rng).expect("setup");

        let public_inputs = public_inputs_for(&circuit);
        let proof = prove(&pk, circuit, &mut rng).expect("prove");

        let accepted = verify(&vk, &public_inputs, &proof).expect("verify");
        assert!(
            accepted,
            "real-fixture proof must verify against its public inputs"
        );
    }

    /// Audit B-1/B-2 S2b: a section-less `dummy()` is NO LONGER
    /// provable — `prove()` must surface the mandatory-binding
    /// rejection rather than emit a forgeable empty-circuit proof.
    /// This is the end-to-end (wrapper-level) proof the vacuity hole
    /// is closed.
    #[test]
    fn prove_rejects_section_less_dummy() {
        let mut rng = ark_std::rand::rngs::StdRng::seed_from_u64(101);
        let (pk, _vk) = setup(&mut rng).expect("setup");
        let result = prove(&pk, NovaVerifierCircuit::dummy(), &mut rng);
        assert!(
            result.is_err(),
            "prove() on a section-less dummy must fail under S2b, got Ok"
        );
    }

    /// Tampered public input must be rejected by verify. Catches
    /// a regression where verify accidentally short-circuits to
    /// `Ok(true)` regardless of input. S2b-prover: real-fixture
    /// proof (the only satisfiable witness post-S2b).
    #[test]
    #[ignore = "S2b-prover: real Nova fixture + /tmp/neptune-bn256-standard.json (expensive)"]
    fn verify_rejects_tampered_public_input() {
        let Some(circuit) = real_provable_circuit() else {
            return;
        };
        let mut rng = ark_std::rand::rngs::StdRng::seed_from_u64(202);
        let (pk, vk) = setup(&mut rng).expect("setup");

        let mut public_inputs = public_inputs_for(&circuit);
        let proof = prove(&pk, circuit, &mut rng).expect("prove");

        // Sanity: untouched verify accepts.
        assert!(verify(&vk, &public_inputs, &proof).expect("verify clean"));

        // Tamper: perturb the first public input.
        public_inputs[0] += Bn254Fr::from(1u64);
        let rejected = verify(&vk, &public_inputs, &proof).expect("verify tampered");
        assert!(
            !rejected,
            "verify must reject a proof against tampered public inputs"
        );
    }

    /// `public_inputs_for` returns exactly the two committed-hash
    /// entries when z0 and zi are empty.
    #[test]
    fn public_inputs_for_empty_state_yields_only_hashes() {
        let circuit =
            NovaVerifierCircuit::new(1, vec![], vec![], Bn254Fr::from(7u64), Bn254Fr::from(11u64));
        let pi = public_inputs_for(&circuit);
        assert_eq!(pi, vec![Bn254Fr::from(7u64), Bn254Fr::from(11u64)]);
    }

    /// Tampering the *second* hash slot also rejects. Existing
    /// tampered-input test only flips slot 0.
    #[test]
    #[ignore = "S2b-prover: real Nova fixture + /tmp/neptune-bn256-standard.json (expensive)"]
    fn verify_rejects_tampered_secondary_hash() {
        let Some(circuit) = real_provable_circuit() else {
            return;
        };
        let mut rng = ark_std::rand::rngs::StdRng::seed_from_u64(404);
        let (pk, vk) = setup(&mut rng).expect("setup");
        let mut public_inputs = public_inputs_for(&circuit);
        let proof = prove(&pk, circuit, &mut rng).expect("prove");
        public_inputs[1] += Bn254Fr::from(99u64);
        assert!(!verify(&vk, &public_inputs, &proof).expect("verify"));
    }

    /// A proof under one setup must not verify against a different
    /// setup's vk.
    #[test]
    #[ignore = "S2b-prover: real Nova fixture + /tmp/neptune-bn256-standard.json (expensive)"]
    fn verify_rejects_proof_against_wrong_vk() {
        let Some(circuit) = real_provable_circuit() else {
            return;
        };
        let mut rng_a = ark_std::rand::rngs::StdRng::seed_from_u64(11);
        let mut rng_b = ark_std::rand::rngs::StdRng::seed_from_u64(22);
        let (pk_a, _vk_a) = setup(&mut rng_a).expect("setup a");
        let (_pk_b, vk_b) = setup(&mut rng_b).expect("setup b");
        let pi = public_inputs_for(&circuit);
        let proof = prove(&pk_a, circuit, &mut rng_a).expect("prove with pk_a");
        assert!(
            !verify(&vk_b, &pi, &proof).expect("verify against wrong vk"),
            "proof under setup A must not verify under setup B's vk"
        );
    }

    /// `public_inputs_for` honours the circuit's allocation order:
    /// [hash_primary, hash_secondary, z0[..], zi[..]]. Pin the
    /// exact layout for a circuit with non-trivial arity so future
    /// refactors don't silently shift the public-input slice.
    #[test]
    fn public_inputs_for_honours_circuit_allocation_order() {
        let circuit = NovaVerifierCircuit::new(
            7,
            vec![Bn254Fr::from(11u64), Bn254Fr::from(13u64)],
            vec![Bn254Fr::from(17u64), Bn254Fr::from(19u64)],
            Bn254Fr::from(2u64),
            Bn254Fr::from(3u64),
        );
        let pi = public_inputs_for(&circuit);
        assert_eq!(
            pi,
            vec![
                Bn254Fr::from(2u64),  // committed_hash_primary
                Bn254Fr::from(3u64),  // committed_hash_secondary
                Bn254Fr::from(11u64), // z0[0]
                Bn254Fr::from(13u64), // z0[1]
                Bn254Fr::from(17u64), // zi[0]
                Bn254Fr::from(19u64), // zi[1]
            ],
        );
    }

    // ─── B-1/B-2 1C §7 step 1 SMOKE TEST: end-to-end Groth16 over
    //     RecursionDeciderCircuit at small n. ─────────────────────────
    //
    // At n=4 bases this is fast enough to run in normal `cargo test`
    // (under a second). The full n_aux=16,384 setup is a separate
    // scheduled Mini-cluster run, NOT a unit test.
    #[test]
    #[allow(deprecated)]
    fn recursion_decider_groth16_roundtrip_n4_smoke() {
        use crate::grumpkin_config::GrumpkinConfig;
        use crate::recursion_decider_circuit::RecursionDeciderCircuit;
        use ark_ec::short_weierstrass::{Projective, SWCurveConfig};
        use ark_ec::CurveGroup;

        // Build n=4 real bases + h via doubling chain (real points,
        // not toy zeros — the circuit bakes these in as constants and
        // setup/prove must agree on them exactly).
        let g = Projective::<GrumpkinConfig>::from(GrumpkinConfig::GENERATOR);
        let g2 = g + g;
        let g3 = g2 + g;
        let g5 = g3 + g2;
        let h = g + g + g + g + g + g + g; // 7G
        let bases: Vec<_> = [g, g2, g3, g5]
            .into_iter()
            .map(|p| p.into_affine())
            .collect();
        let h_aff = h.into_affine();

        let mut rng = ark_std::rand::rngs::StdRng::seed_from_u64(42);

        // Setup keyed over the shape (zero scalars; bases baked in).
        let (pk, vk) = setup_recursion_decider(bases.clone(), h_aff, &mut rng)
            .expect("setup_recursion_decider must succeed");

        // Prove a CONSISTENT non-trivial witness: same bases + h, but
        // real scalars + the matching ck_hat. (If the witness's bases
        // differed from the keyed bases the prove would fail at the
        // CS-shape level — this test pins that contract too.)
        use ark_bn254::Fq as Bn254Fq;
        let scalars = vec![
            Bn254Fq::from(2u64),
            Bn254Fq::from(3u64),
            Bn254Fq::from(5u64),
            Bn254Fq::from(7u64),
        ];
        let blind = Bn254Fq::from(11u64);
        let claimed =
            g * scalars[0] + g2 * scalars[1] + g3 * scalars[2] + g5 * scalars[3] + h * blind;
        let circuit =
            RecursionDeciderCircuit::section_a_only(scalars, bases, blind, h_aff, claimed);
        let proof = prove_recursion_decider(&pk, circuit, &mut rng)
            .expect("prove_recursion_decider must succeed");

        // Section A allocates ZERO new_input calls → no public-inputs
        // to thread. Sections B-D will add (committed_hash_primary,
        // committed_hash_secondary, z0, zi) when wired.
        let public_inputs: Vec<Bn254Fr> = vec![];
        let ok = verify_recursion_decider(&vk, &public_inputs, &proof)
            .expect("verify_recursion_decider must succeed");
        assert!(ok, "Section-A-only Groth16 round-trip must verify");
    }

    /// NON-VACUITY at the GROTH16 LEVEL: tamper the witness's
    /// `claimed_ck_hat` and confirm the Groth16 pipeline rejects.
    /// The CS-level non-vacuity is already pinned by
    /// `recursion_decider_circuit::tests::section_a_wrong_commitment_breaks_cs`;
    /// this test pins the contract END-TO-END through Groth16's
    /// witness assignment + prove, catching any case where the
    /// SNARK layer could mask a CS violation.
    ///
    /// Expected behavior: arkworks Groth16 `prove` on an unsatisfied
    /// CS returns `SynthesisError::Unsatisfiable` (the prover cannot
    /// construct a witness assignment that satisfies the keyed
    /// circuit). If that ever changes (e.g. prove returns a proof
    /// that verify rejects), the test still passes via the
    /// alternate-failure branch below — the contract is "tampered
    /// witness CANNOT round-trip", not the specific failure mode.
    #[test]
    #[allow(deprecated)]
    fn recursion_decider_groth16_tampered_witness_rejected() {
        use crate::grumpkin_config::GrumpkinConfig;
        use crate::recursion_decider_circuit::RecursionDeciderCircuit;
        use ark_bn254::Fq as Bn254Fq;
        use ark_ec::short_weierstrass::{Projective, SWCurveConfig};
        use ark_ec::CurveGroup;

        let g = Projective::<GrumpkinConfig>::from(GrumpkinConfig::GENERATOR);
        let g2 = g + g;
        let g3 = g2 + g;
        let g5 = g3 + g2;
        let h = g + g + g + g + g + g + g;
        let bases: Vec<_> = [g, g2, g3, g5]
            .into_iter()
            .map(|p| p.into_affine())
            .collect();
        let h_aff = h.into_affine();

        let mut rng = ark_std::rand::rngs::StdRng::seed_from_u64(7);
        let (pk, vk) = setup_recursion_decider(bases.clone(), h_aff, &mut rng).expect("setup");

        let scalars = vec![
            Bn254Fq::from(2u64),
            Bn254Fq::from(3u64),
            Bn254Fq::from(5u64),
            Bn254Fq::from(7u64),
        ];
        let blind = Bn254Fq::from(11u64);
        // CORRECT claimed_ck_hat:
        let correct_claimed =
            g * scalars[0] + g2 * scalars[1] + g3 * scalars[2] + g5 * scalars[3] + h * blind;
        // TAMPERED: shift the claim by +G. The MSM binding must
        // refuse to round-trip through Groth16.
        let tampered = correct_claimed + g;

        let bad_circuit =
            RecursionDeciderCircuit::section_a_only(scalars, bases, blind, h_aff, tampered);

        // ark-groth16 0.6 added a defensive `assert!(cs.is_satisfied())`
        // inside `prove` (panics instead of returning Err on
        // unsatisfiable input). 0.5 returned Err. Either is fine for our
        // contract — "tampered witness CANNOT round-trip" — so we accept
        // panic OR Err OR (rare) Ok-but-verify-rejects.
        let bad_circuit_for_panic = bad_circuit.clone();
        let prove_outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut rng_inner = ark_std::rand::rngs::StdRng::seed_from_u64(7);
            prove_recursion_decider(&pk, bad_circuit_for_panic, &mut rng_inner)
        }));
        let _ = bad_circuit;

        match prove_outcome {
            // ark 0.6: assert-panic inside prove. Contract satisfied.
            Err(_panic) => { /* contract satisfied */ }
            // ark 0.5-style: Err from prove. Contract satisfied.
            Ok(Err(_)) => { /* contract satisfied */ }
            // Unexpected: prove succeeded — verify MUST reject.
            Ok(Ok(proof)) => {
                let ok = verify_recursion_decider(&vk, &[], &proof)
                    .expect("verify must not error on tampered proof");
                assert!(
                    !ok,
                    "tampered witness produced a verifying proof — \
                     Section A is VACUOUS at the Groth16 level"
                );
            }
        }
    }

    /// SCALING SMOKE: same pipeline at n=64. Catches any
    /// pipeline-level issue (memory layout, key serialisation,
    /// witness construction) that emerges with more bases before
    /// paying for the full n_aux=16,384 setup (~hours on the Mini).
    /// Per (d)-1: at n=64, total cons ≈ 2,521 + 64·2,533 ≈ 164k —
    /// Groth16 setup should land sub-30 s release-mode on a Mini.
    #[test]
    #[allow(deprecated)]
    fn recursion_decider_groth16_roundtrip_n64_smoke() {
        use crate::grumpkin_config::GrumpkinConfig;
        use crate::recursion_decider_circuit::RecursionDeciderCircuit;
        use ark_bn254::Fq as Bn254Fq;
        use ark_ec::short_weierstrass::{Projective, SWCurveConfig};
        use ark_ec::CurveGroup;

        let n: usize = 64;
        let g = Projective::<GrumpkinConfig>::from(GrumpkinConfig::GENERATOR);
        let h_pt = g * Bn254Fq::from(7u64);

        // Doubling-chain bases (real points, distinct).
        let mut bases = Vec::with_capacity(n);
        let mut cur = g;
        for _ in 0..n {
            bases.push(cur.into_affine());
            cur += g;
        }
        let h_aff = h_pt.into_affine();

        let mut rng = ark_std::rand::rngs::StdRng::seed_from_u64(64);
        let (pk, vk) = setup_recursion_decider(bases.clone(), h_aff, &mut rng).expect("n=64 setup");

        // Pseudo-random scalars from a deterministic seed.
        let scalars: Vec<Bn254Fq> = (0..n)
            .map(|i| Bn254Fq::from((i as u64).wrapping_mul(2654435761) ^ 0xABCD))
            .collect();
        let blind = Bn254Fq::from(11u64);
        // Consistent claimed_ck_hat.
        let mut claimed = h_pt * blind;
        let mut cur = g;
        for s in &scalars {
            claimed += cur * *s;
            cur += g;
        }

        let circuit =
            RecursionDeciderCircuit::section_a_only(scalars, bases, blind, h_aff, claimed);
        let proof = prove_recursion_decider(&pk, circuit, &mut rng).expect("n=64 prove");
        let ok = verify_recursion_decider(&vk, &[], &proof).expect("n=64 verify");
        assert!(ok, "n=64 Section-A Groth16 round-trip must verify");
    }

    /// (e)-1 EIP-197 WIRE-FORMAT ROUND-TRIP for the RecursionDeciderCircuit
    /// proof. The existing `eip197::proof_to_eip197_bytes` was originally
    /// written for `NovaVerifierCircuit` proofs but it operates on the
    /// generic `Proof<Bn254>` type (Groth16 proofs are universal across
    /// BN254 circuits). This test pins that the codec works identically
    /// on the new circuit's proofs — a prerequisite for the EVM round-
    /// trip Foundry test.
    ///
    /// Validates: setup → prove → EIP-197 encode → 256-byte length →
    /// decode → verify accepts the round-tripped proof.
    #[test]
    #[allow(deprecated)]
    fn recursion_decider_groth16_eip197_roundtrip() {
        use crate::eip197::{eip197_bytes_to_proof, proof_to_eip197_bytes, EIP197_PROOF_BYTES};
        use crate::grumpkin_config::GrumpkinConfig;
        use crate::recursion_decider_circuit::RecursionDeciderCircuit;
        use ark_bn254::Fq as Bn254Fq;
        use ark_ec::short_weierstrass::{Projective, SWCurveConfig};
        use ark_ec::CurveGroup;

        let g = Projective::<GrumpkinConfig>::from(GrumpkinConfig::GENERATOR);
        let g2 = g + g;
        let g3 = g2 + g;
        let g5 = g3 + g2;
        let h = g + g + g + g + g + g + g;
        let bases: Vec<_> = [g, g2, g3, g5]
            .into_iter()
            .map(|p| p.into_affine())
            .collect();
        let h_aff = h.into_affine();

        let mut rng = ark_std::rand::rngs::StdRng::seed_from_u64(197);
        let (pk, vk) = setup_recursion_decider(bases.clone(), h_aff, &mut rng).expect("setup");

        let scalars = vec![
            Bn254Fq::from(2u64),
            Bn254Fq::from(3u64),
            Bn254Fq::from(5u64),
            Bn254Fq::from(7u64),
        ];
        let blind = Bn254Fq::from(11u64);
        let claimed =
            g * scalars[0] + g2 * scalars[1] + g3 * scalars[2] + g5 * scalars[3] + h * blind;
        let circuit =
            RecursionDeciderCircuit::section_a_only(scalars, bases, blind, h_aff, claimed);

        let proof = prove_recursion_decider(&pk, circuit, &mut rng).expect("prove");

        // EIP-197 round-trip: encode → 256 bytes → decode → equal proof.
        let bytes = proof_to_eip197_bytes(&proof);
        assert_eq!(bytes.len(), EIP197_PROOF_BYTES, "encoded must be 256 B");
        assert_eq!(bytes.len(), 256, "EIP197_PROOF_BYTES must be 256");

        let decoded = eip197_bytes_to_proof(&bytes).expect("decode round-trip");

        // Re-encode the decoded proof; bytes must be byte-identical.
        let bytes_after = proof_to_eip197_bytes(&decoded);
        assert_eq!(
            bytes, bytes_after,
            "EIP-197 encode↔decode must be byte-stable on round-trip"
        );

        // Decoded proof must still pass Groth16 verify.
        let ok = verify_recursion_decider(&vk, &[], &decoded).expect("verify of decoded proof");
        assert!(ok, "decoded EIP-197 proof must verify against the same vk");

        // Print the wire bytes as hex for downstream Foundry / EVM
        // test fixture consumption.
        let hex_str: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
        eprintln!("EIP197_WIRE_HEX = 0x{hex_str}");
    }

    /// Section B END-TO-END: chains the off-chain adapter through
    /// the Groth16-wrap pipeline.
    ///
    /// Flow:
    ///   1. Build (pp, rs) via the shared-pp fixture helper.
    ///   2. assemble_section_b_pi_bundle(pp, rs, num_steps, z0)
    ///      → SectionBPiBundle (verify-then-emit gate).
    ///   3. Convert bundle → SectionBPublicInputs.
    ///   4. setup_recursion_decider_with_b_interface(bases, h,
    ///      pi_arity, rng) → (pk, vk) at Section A + Section B PI shape.
    ///   5. Build RecursionDeciderCircuit::section_a_with_b_interface
    ///      (Section A's consistent witness + the Section B PI bundle).
    ///   6. prove_recursion_decider → Groth16 proof.
    ///   7. verify_recursion_decider with section_b_public_inputs_slice
    ///      → must verify (Section A binds, Section B PIs flow through).
    ///
    /// This validates the delegation architecture end-to-end:
    ///   off-chain adapter (verify gate) → Groth16-wrap with PI
    ///   layout → on-chain-shape verifier accepts.
    #[test]
    #[allow(deprecated)]
    fn recursion_decider_section_b_end_to_end_smoke() {
        use crate::grumpkin_config::GrumpkinConfig;
        use crate::l_u_secondary_extract::assemble_section_b_pi_bundle;
        use crate::recursion_decider_circuit::RecursionDeciderCircuit;
        use crate::recursive_snark_fixture::{Scalar1, TrivialIncrementCircuit, E1, E2};
        use ark_ec::short_weierstrass::{Projective, SWCurveConfig};
        use ark_ec::CurveGroup;
        use ff::Field;
        use nova_snark::nova::{PublicParams, RecursiveSNARK};
        use nova_snark::provider::hyperkzg::EvaluationEngine;
        use nova_snark::provider::ipa_pc::EvaluationEngine as IpaEE;
        use nova_snark::spartan::ppsnark::RelaxedR1CSSNARK;
        use nova_snark::traits::snark::RelaxedR1CSSNARKTrait;

        // ── 1. Build (pp, rs) with the same pp instance ─────────────
        let circuit = TrivialIncrementCircuit;
        type S1 = RelaxedR1CSSNARK<E1, EvaluationEngine<E1>>;
        type S2 = RelaxedR1CSSNARK<E2, IpaEE<E2>>;
        let pp = PublicParams::<E1, E2, TrivialIncrementCircuit>::setup(
            &circuit,
            &*S1::ck_floor(),
            &*S2::ck_floor(),
        )
        .expect("pp setup");
        let z0: Vec<Scalar1> = vec![Scalar1::ZERO];
        let mut rs = RecursiveSNARK::<E1, E2, TrivialIncrementCircuit>::new(&pp, &circuit, &z0)
            .expect("rs new");
        for _ in 0..2 {
            rs.prove_step(&pp, &circuit).expect("prove_step");
        }

        // ── 2. Off-chain adapter (verify-then-emit) ─────────────────
        let z0_ark = vec![Bn254Fr::from(0u64)];
        let bundle = assemble_section_b_pi_bundle(&pp, &rs, 2, &z0_ark).expect("assemble");
        let pi_arity = bundle.z0.len();
        assert_eq!(pi_arity, 1, "TrivialIncrementCircuit z0 arity = 1");

        // ── 3. Convert bundle → in-circuit PIs ──────────────────────
        let section_b_pis = bundle.into_section_b_pis();
        let expected_pi_count = section_b_pis.pi_count();
        assert_eq!(expected_pi_count, 9 + 1 + 1, "11 PIs at arity 1");

        // ── 4. Set up Groth16 with Section A + Section B interface ──
        let g = Projective::<GrumpkinConfig>::from(GrumpkinConfig::GENERATOR);
        let g2 = g + g;
        let g3 = g2 + g;
        let g5 = g3 + g2;
        let h = g + g + g + g + g + g + g;
        let bases: Vec<_> = [g, g2, g3, g5]
            .into_iter()
            .map(|p| p.into_affine())
            .collect();
        let h_aff = h.into_affine();

        let mut rng = ark_std::rand::rngs::StdRng::seed_from_u64(0xB);
        let (pk, vk) =
            setup_recursion_decider_with_b_interface(bases.clone(), h_aff, pi_arity, &mut rng)
                .expect("setup");

        // ── 5. Build the prover circuit with consistent Section A
        //       witness + the Section B PI bundle ─────────────────────
        use ark_bn254::Fq as Bn254Fq;
        let scalars = vec![
            Bn254Fq::from(2u64),
            Bn254Fq::from(3u64),
            Bn254Fq::from(5u64),
            Bn254Fq::from(7u64),
        ];
        let blind = Bn254Fq::from(11u64);
        let claimed =
            g * scalars[0] + g2 * scalars[1] + g3 * scalars[2] + g5 * scalars[3] + h * blind;
        let circuit_ab = RecursionDeciderCircuit::section_a_with_b_interface(
            scalars,
            bases,
            blind,
            h_aff,
            claimed,
            section_b_pis.clone(),
        );

        // ── 6. Prove ────────────────────────────────────────────────
        let proof = prove_recursion_decider(&pk, circuit_ab, &mut rng).expect("prove");

        // ── 7. Verify with the PI slice ─────────────────────────────
        let pis = section_b_public_inputs_slice(&section_b_pis);
        assert_eq!(
            pis.len(),
            expected_pi_count,
            "PI slice length must match pi_count()"
        );

        let ok = verify_recursion_decider(&vk, &pis, &proof).expect("verify call");
        assert!(
            ok,
            "Section B end-to-end Groth16 round-trip must verify with the full PI bundle"
        );
    }

    /// (d)-4 PRODUCTION-SCALE SETUP+PROVE+VERIFY at n_aux=16,384 —
    /// the heavy run that validates the predicted ~41.5M-cons /
    /// ~5.6 GB-memory pipeline on the Mini cluster. Reports
    /// per-phase timing for the dossier. `#[ignore]`: heavy
    /// (estimated 10-60 min setup + 10-30 min prove on a Mini),
    /// run via `--ignored --nocapture`.
    #[test]
    #[ignore = "(d)-4 production-scale Groth16 setup at n_aux=16384 (heavy, Mini)"]
    #[allow(deprecated)]
    fn recursion_decider_groth16_full_n_aux_16384() {
        use crate::grumpkin_config::GrumpkinConfig;
        use crate::recursion_decider_circuit::RecursionDeciderCircuit;
        use ark_bn254::Fq as Bn254Fq;
        use ark_ec::short_weierstrass::{Projective, SWCurveConfig};
        use ark_ec::CurveGroup;
        use std::time::Instant;

        let n: usize = 16_384;
        let g = Projective::<GrumpkinConfig>::from(GrumpkinConfig::GENERATOR);
        let h_pt = g * Bn254Fq::from(7u64);
        let h_aff = h_pt.into_affine();

        // Doubling-chain bases.
        let t_bases = Instant::now();
        let mut bases = Vec::with_capacity(n);
        let mut cur = g;
        for _ in 0..n {
            bases.push(cur.into_affine());
            cur += g;
        }
        eprintln!("D4_BASES n={n} elapsed={:?}", t_bases.elapsed());

        let mut rng = ark_std::rand::rngs::StdRng::seed_from_u64(16_384);

        let t_setup = Instant::now();
        let (pk, vk) =
            setup_recursion_decider(bases.clone(), h_aff, &mut rng).expect("full-n setup");
        let setup_elapsed = t_setup.elapsed();
        eprintln!("D4_SETUP n={n} elapsed={setup_elapsed:?}");

        // Consistent witness.
        let t_witness = Instant::now();
        let scalars: Vec<Bn254Fq> = (0..n)
            .map(|i| Bn254Fq::from((i as u64).wrapping_mul(2654435761) ^ 0xCAFE))
            .collect();
        let blind = Bn254Fq::from(13u64);
        let mut claimed = h_pt * blind;
        let mut cur = g;
        for s in &scalars {
            claimed += cur * *s;
            cur += g;
        }
        eprintln!("D4_WITNESS_ASSEMBLY elapsed={:?}", t_witness.elapsed());

        let circuit =
            RecursionDeciderCircuit::section_a_only(scalars, bases, blind, h_aff, claimed);

        let t_prove = Instant::now();
        let proof = prove_recursion_decider(&pk, circuit, &mut rng).expect("full-n prove");
        let prove_elapsed = t_prove.elapsed();
        eprintln!("D4_PROVE n={n} elapsed={prove_elapsed:?}");

        let t_verify = Instant::now();
        let ok = verify_recursion_decider(&vk, &[], &proof).expect("full-n verify");
        eprintln!("D4_VERIFY elapsed={:?}", t_verify.elapsed());
        assert!(ok, "n=16384 Section-A Groth16 round-trip must verify");

        eprintln!("D4_TOTAL_PHASES setup={setup_elapsed:?} prove={prove_elapsed:?}");
    }
}
