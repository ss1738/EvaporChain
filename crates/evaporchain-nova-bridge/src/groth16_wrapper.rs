//! Phase 2.4 — Groth16-on-BN254 setup / prove / verify wrappers
//! around [`crate::NovaVerifierCircuit`].
//!
//! # What lands here
//!
//! End-to-end pipeline plumbing: pick the Groth16 backend, wire it
//! to the verifier circuit type, expose three operator-friendly
//! entry points (`setup`, `prove`, `verify`). When Sections 2 and 3
//! fill in the actual Nova-verification constraints, this module
//! does NOT change — the SNARK trait is structurally circuit-shape
//! agnostic, so as long as the public-input arity stays consistent
//! between `setup` and `prove`, additional constraints in
//! `generate_constraints` just get covered by the same `pk`/`vk`
//! pair.
//!
//! # Why a separate module
//!
//! Three responsibilities live here that don't belong in the
//! verifier circuit itself:
//!
//!  - **Trusted-setup key generation.** The setup output (`pk`,
//!    `vk`) is shape-derived from the *dummy* circuit
//!    instance — so the dummy must allocate the same number of
//!    public inputs / witness wires as any real circuit will.
//!    [`NovaVerifierCircuit::dummy`] guarantees this by using
//!    `|z0| = |zi| = 1`; if the chain's `RealBlockCircuit` arity
//!    changes (currently 8 per `evaporchain-proving/src/nova.rs`),
//!    setup must be re-run.
//!
//!  - **Proof generation against a circuit + witness.** Returns
//!    an arkworks `Proof<Bn254>` — three group elements totalling
//!    256 bytes uncompressed in EIP-197 form. This is the artifact
//!    the L1 verifier consumes.
//!
//!  - **Verification helpers.** Both the un-prepared `verify` (for
//!    one-shot checks) and the prepared-vk variant (for repeated
//!    verifications on the same key) are exposed; chain code can
//!    pick whichever fits its workload.
//!
//! # What's NOT in this module
//!
//! - **Multi-participant trusted setup ceremony.** `setup` here
//!   runs a one-shot `circuit_specific_setup` with a caller-supplied
//!   RNG. That's appropriate for tests and for reproducible
//!   single-party setups, but a mainnet ceremony requires the
//!   BGM17 / Phase-2 contribute-and-verify protocol, which is
//!   external tooling. The ceremony harness is a separate sprint.
//!
//! - **EIP-197 byte layout.** Arkworks `Proof<Bn254>` serializes
//!   via `ark-serialize`; conversion to the 256-byte EIP-197
//!   uncompressed layout that L1's `EIP197Pairing` precompile
//!   reads happens at the bridge → contract boundary. See
//!   `ethereum-bridge/circuits/src/circuit_v2.rs` for the
//!   precedent in the legacy IPA-in-Groth16 path.

use ark_bn254::{Bn254, Fr};
use ark_groth16::{Groth16, PreparedVerifyingKey, Proof, ProvingKey, VerifyingKey};
use ark_relations::r1cs::SynthesisError;
use ark_snark::SNARK;
use ark_std::rand::{CryptoRng, Rng};

use crate::verifier_circuit::NovaVerifierCircuit;

/// Bundled output of [`setup`]. The `pk` is multi-megabyte; the
/// `vk` is small (~256 bytes worth of group elements). Most chain
/// code only retains `vk` after setup.
pub struct WrapperKeys {
    /// Proving key — needed by the prover.
    pub pk: ProvingKey<Bn254>,
    /// Verifying key — needed by the on-chain verifier (and
    /// by [`verify`] off-chain).
    pub vk: VerifyingKey<Bn254>,
}

/// Run Groth16 trusted-setup against a fresh
/// [`NovaVerifierCircuit::dummy`] instance and return both keys.
///
/// **Shape contract.** The dummy instance must allocate the same
/// number of public inputs (`2 + |z0| + |zi|`) as any real
/// instance that will be proved against the returned `pk`. The
/// dummy is fixed at arity 1; real instances must match.
///
/// **RNG.** The caller picks the RNG. Tests pass a seeded
/// `StdRng` so the run is reproducible; production deployments
/// pass an OS RNG. A multi-participant ceremony would replace
/// this call entirely with a contribution-protocol output.
pub fn setup<R: Rng + CryptoRng>(rng: &mut R) -> Result<WrapperKeys, SynthesisError> {
    let circuit = NovaVerifierCircuit::dummy();
    let (pk, vk) = Groth16::<Bn254>::circuit_specific_setup(circuit, rng)?;
    Ok(WrapperKeys { pk, vk })
}

/// Produce a Groth16 proof that the given `circuit` instance
/// satisfies its constraints.
pub fn prove<R: Rng + CryptoRng>(
    pk: &ProvingKey<Bn254>,
    circuit: NovaVerifierCircuit,
    rng: &mut R,
) -> Result<Proof<Bn254>, SynthesisError> {
    Groth16::<Bn254>::prove(pk, circuit, rng)
}

/// Verify a Groth16 proof against the un-prepared verifying key.
/// One-shot use — for repeated verifications on the same `vk`,
/// prefer [`prepare_vk`] + [`verify_prepared`].
pub fn verify(
    vk: &VerifyingKey<Bn254>,
    public_inputs: &[Fr],
    proof: &Proof<Bn254>,
) -> Result<bool, SynthesisError> {
    Groth16::<Bn254>::verify(vk, public_inputs, proof)
}

/// Pre-process a verifying key for repeated verification.
/// `PreparedVerifyingKey<Bn254>` precomputes the pairing inputs
/// that don't depend on the proof.
pub fn prepare_vk(vk: &VerifyingKey<Bn254>) -> PreparedVerifyingKey<Bn254> {
    Groth16::<Bn254>::process_vk(vk).expect("process_vk does not fail for valid VerifyingKey")
}

/// Verify a Groth16 proof against a prepared verifying key.
/// Faster than [`verify`] when the same `vk` is used repeatedly.
pub fn verify_prepared(
    pvk: &PreparedVerifyingKey<Bn254>,
    public_inputs: &[Fr],
    proof: &Proof<Bn254>,
) -> Result<bool, SynthesisError> {
    Groth16::<Bn254>::verify_with_processed_vk(pvk, public_inputs, proof)
}

/// Assemble the public-input vector that [`verify`] consumes,
/// in the order [`NovaVerifierCircuit::generate_constraints`]
/// allocates them: `[committed_hash_primary, committed_hash_secondary,
/// z0[..], zi[..]]`.
///
/// Stable across all Phase 2.2 sub-steps — Section 2 adds *constraints*,
/// not *additional public inputs*. The committed hashes are already
/// public inputs in the skeleton.
pub fn public_inputs_in_alloc_order(circuit: &NovaVerifierCircuit) -> Vec<Fr> {
    let mut v = Vec::with_capacity(2 + circuit.z0.len() + circuit.zi.len());
    v.push(circuit.committed_hash_primary);
    v.push(circuit.committed_hash_secondary);
    v.extend_from_slice(&circuit.z0);
    v.extend_from_slice(&circuit.zi);
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_std::rand::SeedableRng;
    use ark_std::rand::rngs::StdRng;

    /// Pin that the trusted-setup pipeline runs to completion on
    /// the dummy circuit and produces non-empty keys.
    #[test]
    fn setup_succeeds_on_dummy_circuit() {
        let mut rng = StdRng::seed_from_u64(0xa17b_5d_5e7_70u64);
        let keys = setup(&mut rng).expect("setup");
        // `pk.beta_g1`, `pk.delta_g1`, `pk.vk.alpha_g1`, etc. are
        // all populated as part of `circuit_specific_setup`.
        // Exact field accessors vary by ark-groth16 minor version;
        // shape sanity is enough — `vk.gamma_abc_g1` has one entry
        // per public input PLUS the constant.
        assert_eq!(
            keys.vk.gamma_abc_g1.len(),
            public_inputs_in_alloc_order(&NovaVerifierCircuit::dummy()).len() + 1,
            "vk.gamma_abc_g1 size = (#public inputs) + 1"
        );
    }

    /// Pin the prove → verify round-trip end-to-end on a
    /// non-dummy circuit instance. With Sections 2 + 3 still
    /// unimplemented, the circuit allocates 4 public-input
    /// scalars + carries no constraints — every well-formed
    /// witness satisfies it. This test therefore validates the
    /// *pipeline plumbing*, not the verifier's semantic content.
    /// When Section 2 lands, this test continues to pass without
    /// modification; when Section 2 + 3 are wired, the test
    /// gains semantic teeth (an inconsistent committed_hash → wrong
    /// witness → SynthesisError at prove time).
    #[test]
    fn prove_verify_round_trip() {
        let mut rng = StdRng::seed_from_u64(0xa17b_5d_5e7_71u64);
        let keys = setup(&mut rng).expect("setup");

        // Real-shape circuit — arity-1 to match the dummy used at
        // setup. Hash values are arbitrary witness scalars; with
        // no Section-2 constraints in place, any values satisfy
        // the (empty) hash-equality check.
        let circuit = NovaVerifierCircuit::new(
            5,
            vec![Fr::from(7u64)],
            vec![Fr::from(11u64)],
            Fr::from(0x1234abcdu64),
            Fr::from(0x5678ef01u64),
        );
        let public_inputs = public_inputs_in_alloc_order(&circuit);

        let proof = prove(&keys.pk, circuit, &mut rng).expect("prove");
        let ok = verify(&keys.vk, &public_inputs, &proof).expect("verify");
        assert!(ok, "valid proof must verify");
    }

    /// Pin that the verifier rejects a proof whose public inputs
    /// have been mutated post-hoc. This is the canonical Groth16
    /// soundness sanity check — without it, an attacker could
    /// substitute any z0/zi/hash values and the verifier would
    /// accept.
    #[test]
    fn verify_rejects_mutated_public_input() {
        let mut rng = StdRng::seed_from_u64(0xa17b_5d_5e7_72u64);
        let keys = setup(&mut rng).expect("setup");

        let circuit = NovaVerifierCircuit::new(
            5,
            vec![Fr::from(7u64)],
            vec![Fr::from(11u64)],
            Fr::from(0x1234abcdu64),
            Fr::from(0x5678ef01u64),
        );
        let mut public_inputs = public_inputs_in_alloc_order(&circuit);
        let proof = prove(&keys.pk, circuit, &mut rng).expect("prove");

        // Sanity: original inputs verify.
        assert!(
            verify(&keys.vk, &public_inputs, &proof).expect("verify"),
            "valid proof should verify before mutation"
        );

        // Now flip one bit on the committed_hash_primary input.
        public_inputs[0] += Fr::from(1u64);
        let result = verify(&keys.vk, &public_inputs, &proof).expect("verify call itself");
        assert!(!result, "mutated public input must reject");
    }

    /// Pin that the prepared-vk variant returns the same accept /
    /// reject decisions as the un-prepared variant. Catches any
    /// API drift between the two code paths.
    #[test]
    fn prepared_and_unprepared_verify_agree() {
        let mut rng = StdRng::seed_from_u64(0xa17b_5d_5e7_73u64);
        let keys = setup(&mut rng).expect("setup");
        let pvk = prepare_vk(&keys.vk);

        let circuit = NovaVerifierCircuit::new(
            3,
            vec![Fr::from(42u64)],
            vec![Fr::from(99u64)],
            Fr::from(0xaaaa_bbbb_u64),
            Fr::from(0xcccc_dddd_u64),
        );
        let public_inputs = public_inputs_in_alloc_order(&circuit);
        let proof = prove(&keys.pk, circuit, &mut rng).expect("prove");

        let ok_unprep = verify(&keys.vk, &public_inputs, &proof).expect("verify");
        let ok_prep = verify_prepared(&pvk, &public_inputs, &proof).expect("verify_prepared");
        assert_eq!(ok_unprep, ok_prep);
        assert!(ok_unprep);
    }
}
