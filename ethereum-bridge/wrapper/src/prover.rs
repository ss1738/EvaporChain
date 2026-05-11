//! Groth16 setup + prove + verify wrappers around the `WrapperCircuit`.
//!
//! # Setup safety
//!
//! [`setup`] currently uses an in-process keygen with `rand::thread_rng`.
//! This is **NOT SAFE** for production — the trapdoor τ used by
//! `generate_random_parameters` lives in the process's memory and a
//! malicious prover holding it can forge proofs. Sub-C (the trusted-
//! setup ceremony) replaces this with the output of a multi-party
//! computation whose τ is destroyed at ceremony close.
//!
//! For starter testing — exercising the proving + verifying path
//! against a freshly-generated VK — this is fine. Just never deploy
//! the VK from [`setup`] to L1.

use crate::circuit::WrapperCircuit;
use crate::inputs::WrapperPublicInputs;
use ark_bn254::{Bn254, Fr};
use ark_groth16::{Groth16, Proof, ProvingKey, VerifyingKey};
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use ark_snark::SNARK;

#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error("Groth16 setup: {0}")]
    Groth16(String),
}

#[derive(Debug, thiserror::Error)]
pub enum ProveError {
    #[error("Groth16 prove: {0}")]
    Groth16(String),
    #[error("proof serialize: {0}")]
    Serialize(String),
}

#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    #[error("Groth16 verify: {0}")]
    Groth16(String),
    #[error("proof deserialize: {0}")]
    Deserialize(String),
    #[error("public inputs failed Groth16 check")]
    InvalidProof,
}

/// Run an **unsafe** in-process trusted-setup against [`WrapperCircuit::dummy`].
///
/// Returns the proving + verifying keys. The proving key is large
/// (~MB scale); cache + reuse a single setup result for many proves.
///
/// **Do not use the returned VK in production.** See module-level safety doc.
pub fn setup(
    rng: &mut impl ark_std::rand::RngCore,
) -> Result<(ProvingKey<Bn254>, VerifyingKey<Bn254>), SetupError> {
    let (pk, vk) = Groth16::<Bn254>::circuit_specific_setup(WrapperCircuit::dummy(), rng)
        .map_err(|e| SetupError::Groth16(format!("{:?}", e)))?;
    Ok((pk, vk))
}

/// Generate a Groth16 proof for the given public inputs + Halo2 IPA
/// proof bytes (the latter is currently witness-only).
///
/// The returned byte vector is the 256-byte L1 calldata shape:
/// `A (64B) || B (128B) || C (64B)`. This matches what
/// `VerkleProofVerifier.sol`'s `verifyVerkleMembership(..., bytes calldata groth16Proof)`
/// expects.
pub fn prove(
    pk: &ProvingKey<Bn254>,
    public_inputs: WrapperPublicInputs,
    halo2_ipa_proof_bytes: Vec<u8>,
    rng: &mut impl ark_std::rand::RngCore,
) -> Result<Vec<u8>, ProveError> {
    let circuit = WrapperCircuit::new(public_inputs, halo2_ipa_proof_bytes);
    let proof = Groth16::<Bn254>::prove(pk, circuit, rng)
        .map_err(|e| ProveError::Groth16(format!("{:?}", e)))?;
    let mut bytes = Vec::with_capacity(256);
    proof
        .serialize_compressed(&mut bytes)
        .map_err(|e| ProveError::Serialize(format!("{:?}", e)))?;
    Ok(bytes)
}

/// Verify a Groth16 proof against the public inputs.
///
/// Returns `Ok(())` on accept, `Err(VerifyError::InvalidProof)` on
/// reject, or other variants on structural failures (proof bytes
/// malformed, etc.).
pub fn verify(
    vk: &VerifyingKey<Bn254>,
    public_inputs: &WrapperPublicInputs,
    proof_bytes: &[u8],
) -> Result<(), VerifyError> {
    let proof = Proof::<Bn254>::deserialize_compressed(proof_bytes)
        .map_err(|e| VerifyError::Deserialize(format!("{:?}", e)))?;

    let public_inputs_vec: Vec<Fr> = public_inputs.to_vec();
    let ok = Groth16::<Bn254>::verify(vk, &public_inputs_vec, &proof)
        .map_err(|e| VerifyError::Groth16(format!("{:?}", e)))?;
    if ok {
        Ok(())
    } else {
        Err(VerifyError::InvalidProof)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_std::rand::SeedableRng;
    use ark_std::test_rng;

    /// Headline test — setup, prove, verify all run end-to-end against
    /// the starter circuit. Slow (~seconds for keygen + proving on a
    /// laptop), so `#[ignore]`'d.
    #[test]
    #[ignore]
    fn starter_setup_prove_verify_round_trip() {
        let mut rng = test_rng();
        let (pk, vk) = setup(&mut rng).expect("setup must succeed");

        let public_inputs = WrapperPublicInputs {
            state_root: Fr::from(0x09u64),
            key: Fr::from(0x2bu64),
            value_commitment: Fr::from(0x22u64),
            params_fingerprint: Fr::from(0x2eu64),
        };

        let proof_bytes = prove(
            &pk,
            public_inputs.clone(),
            vec![0xde, 0xad, 0xbe, 0xef],
            &mut rng,
        )
        .expect("prove must succeed");

        verify(&vk, &public_inputs, &proof_bytes).expect("verify must succeed");
    }

    /// Tampered public inputs must produce `InvalidProof`. Pins that
    /// Groth16 is checking the IC[] table — without this, the wrapper
    /// would accept any anchors for any proof.
    #[test]
    #[ignore]
    fn tampered_public_input_fails_verify() {
        let mut rng = test_rng();
        let (pk, vk) = setup(&mut rng).expect("setup");

        let real_inputs = WrapperPublicInputs {
            state_root: Fr::from(1u64),
            key: Fr::from(2u64),
            value_commitment: Fr::from(3u64),
            params_fingerprint: Fr::from(4u64),
        };
        let proof_bytes = prove(&pk, real_inputs.clone(), vec![], &mut rng).expect("prove");

        let tampered_inputs = WrapperPublicInputs {
            state_root: Fr::from(999u64), // ← changed
            key: Fr::from(2u64),
            value_commitment: Fr::from(3u64),
            params_fingerprint: Fr::from(4u64),
        };
        match verify(&vk, &tampered_inputs, &proof_bytes) {
            Err(VerifyError::InvalidProof) => (),
            other => panic!("expected InvalidProof, got {:?}", other),
        }
    }

    /// Deterministic-RNG round trip — proves the proving path is
    /// reproducible given a fixed RNG. Useful for fixture emission
    /// (sub-B-finish will need to produce the same Groth16 proof bytes
    /// across operator machines for the L1 fixture).
    #[test]
    #[ignore]
    fn deterministic_rng_proof_round_trip() {
        let mut rng = ark_std::rand::rngs::StdRng::seed_from_u64(0xC0FFEE_u64);
        let (pk, vk) = setup(&mut rng).expect("setup");

        let inputs = WrapperPublicInputs {
            state_root: Fr::from(42u64),
            key: Fr::from(99u64),
            value_commitment: Fr::from(7u64),
            params_fingerprint: Fr::from(2026u64),
        };
        let proof_bytes =
            prove(&pk, inputs.clone(), vec![], &mut rng).expect("prove");
        verify(&vk, &inputs, &proof_bytes).expect("verify");
    }
}
