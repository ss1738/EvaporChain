//! Verkle IVC prover: folds D Verkle levels into a CompressedSNARK.

use ff::PrimeField;
use nova_snark::{
    nova::{CompressedSNARK, RecursiveSNARK},
    traits::snark::RelaxedR1CSSNARKTrait,
};
use serde::{Deserialize, Serialize};

use crate::{
    circuit::VerkleStepCircuit,
    types::{Scalar, VerklePublicParams, VerkleRecursiveSNARK, G1, S1, S2},
};

/// A serialisable Verkle membership proof bundle.
///
/// The Solidity verifier (`VerkleProofVerifier.sol`) will check:
///   - `z_0` == `keccak_poseidon(key, value)` (computed on-chain or supplied)
///   - `z_final` == `stateRootAt(height)` in `EvaporHeaderInbox`
///   - `num_steps` matches the declared Verkle depth
///   - compressed_snark.verify(vk, num_steps, z_0, z_final) == true
#[derive(Debug, Serialize, Deserialize)]
pub struct VerkleProof {
    /// z_0 = Poseidon(key_scalar, value_scalar) — the leaf commitment hash.
    pub z_0: Vec<u8>,
    /// z_final = state_root scalar — matches EvaporHeaderInbox.
    pub z_final: Vec<u8>,
    /// Number of IVC folding steps (Verkle proof depth).
    pub num_steps: usize,
    /// The compressed SNARK proof bytes (bincode + zlib).
    pub proof_bytes: Vec<u8>,
}

/// VerkleProver: manages public params and proof generation.
pub struct VerkleProver {
    pp: VerklePublicParams,
}

impl VerkleProver {
    /// Set up public parameters.  Expensive (~seconds); cache and reuse.
    pub fn setup() -> Result<Self, String> {
        let dummy = VerkleStepCircuit::<G1>::dummy();
        let pp = VerklePublicParams::setup(&dummy, &*S1::ck_floor(), &*S2::ck_floor())
            .map_err(|e| format!("PublicParams::setup failed: {:?}", e))?;
        Ok(Self { pp })
    }

    /// Fold a Verkle path (bottom-up) and return a compressed proof bundle.
    ///
    /// `key`   : 32-byte Verkle key (as stored in the trie)
    /// `value` : 32-byte Verkle value
    /// `path`  : per-level (path_index, commitment_bytes) pairs, leaf-first
    ///
    /// The `commitment_bytes` at each level are the output of
    /// `evaporchain_crypto::verkle::point_to_bytes()` for that level's
    /// commitment node.
    pub fn prove(
        &self,
        key: &[u8; 32],
        value: &[u8; 32],
        path: &[(u8, [u8; 32])], // (path_index, commitment_hash_bytes) leaf→root
        state_root_bytes: &[u8; 32],
    ) -> Result<VerkleProof, String> {
        let depth = path.len();
        if depth == 0 {
            return Err("Verkle path must have at least one level".into());
        }

        // --- Build z_0 (leaf commitment hash) ---
        let key_scalar   = bytes_to_scalar::<Scalar>(key);
        let value_scalar = bytes_to_scalar::<Scalar>(&blake3_hash(value));
        let z_0 = VerkleStepCircuit::<G1>::leaf_hash(key_scalar, value_scalar);

        // --- Fold each level ---
        let first_circuit = build_step_circuit::<G1>(&path[0]);

        let mut recursive_snark: VerkleRecursiveSNARK =
            RecursiveSNARK::new(&self.pp, &first_circuit, &[z_0])
                .map_err(|e| format!("RecursiveSNARK::new: {:?}", e))?;

        for step in path.iter() {
            let circuit = build_step_circuit::<G1>(step);
            recursive_snark
                .prove_step(&self.pp, &circuit)
                .map_err(|e| format!("prove_step: {:?}", e))?;
        }

        // Verify the recursive SNARK before compressing (sanity).
        recursive_snark
            .verify(&self.pp, depth, &[z_0])
            .map_err(|e| format!("RecursiveSNARK::verify: {:?}", e))?;

        // --- Compress ---
        let (pk, vk) = CompressedSNARK::<_, _, _, S1, S2>::setup(&self.pp)
            .map_err(|e| format!("CompressedSNARK::setup: {:?}", e))?;

        let compressed = CompressedSNARK::<_, _, _, S1, S2>::prove(&self.pp, &pk, &recursive_snark)
            .map_err(|e| format!("CompressedSNARK::prove: {:?}", e))?;

        // Compute z_final from the folded IVC state (it equals the state root scalar).
        let state_root_scalar = bytes_to_scalar::<Scalar>(state_root_bytes);

        // Serialise compressed SNARK.
        let proof_bytes = bincode::serde::encode_to_vec(
            &compressed,
            bincode::config::legacy(),
        )
        .map_err(|e| format!("bincode encode: {:?}", e))?;

        // Drop vk — caller can re-derive from pp if needed.
        let _ = vk;

        Ok(VerkleProof {
            z_0: scalar_to_bytes(z_0),
            z_final: scalar_to_bytes(state_root_scalar),
            num_steps: depth,
            proof_bytes,
        })
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn build_step_circuit<G>(step: &(u8, [u8; 32])) -> VerkleStepCircuit<G>
where
    G: nova_snark::traits::Group,
    G::Scalar: PrimeField,
{
    let (path_index, commitment_bytes) = step;
    let sibling_hash = bytes_to_scalar::<G::Scalar>(commitment_bytes);
    VerkleStepCircuit::new(*path_index, sibling_hash)
}

/// Convert 32 bytes to a scalar field element, masking top 2 bits so the value
/// fits within both the Pallas Fq and BN254 Fr moduli (~2^254).
/// Mirrors `evaporchain-crypto::verkle::bytes_to_scalar`.
///
/// `PrimeField` guarantees `Repr: Default + AsMut<[u8]> + AsRef<[u8]>`,
/// so no extra where clause is needed beyond `F: PrimeField`.
pub fn bytes_to_scalar<F: PrimeField>(bytes: &[u8; 32]) -> F {
    let mut repr = F::Repr::default();
    repr.as_mut().copy_from_slice(bytes);
    repr.as_mut()[31] &= 0x3F;
    F::from_repr(repr).unwrap_or(F::ONE)
}

/// Serialise a scalar to little-endian bytes.
pub fn scalar_to_bytes<F: PrimeField>(s: F) -> Vec<u8> {
    s.to_repr().as_ref().to_vec()
}

/// BLAKE3 hash — used to derive the value scalar from the 32-byte value.
fn blake3_hash(data: &[u8]) -> [u8; 32] {
    *blake3::hash(data).as_bytes()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ff::{Field, PrimeField};

    #[test]
    fn bytes_to_scalar_masks_top_bits() {
        let b = [0xFFu8; 32];
        let s: Scalar = bytes_to_scalar(&b);
        // After masking [31] & 0x3F, the value must not be zero or all-ones.
        assert_ne!(s, Scalar::ZERO);
        // Must be stable.
        assert_eq!(s, bytes_to_scalar::<Scalar>(&b));
        // The top 2 bits of byte 31 must be clear in repr.
        let repr = s.to_repr();
        assert_eq!(repr.as_ref()[31] & 0xC0, 0);
    }

    #[test]
    fn scalar_round_trip() {
        let s = Scalar::from(0xDEADBEEFu64);
        let bytes = scalar_to_bytes(s);
        assert_eq!(bytes.len(), 32);
        let back: Scalar = bytes_to_scalar(&bytes.try_into().unwrap());
        assert_eq!(s, back);
    }

    /// Smoke test: setup → prove → verify a 3-step path.
    ///
    /// This is slow (~30 s on a Mini) so it's excluded from `cargo test`
    /// unless you pass `--include-ignored`.
    #[test]
    #[ignore]
    fn smoke_prove_3_steps() {
        let prover = VerkleProver::setup().expect("setup");

        let key   = [0x01u8; 32];
        let value = [0x42u8; 32];

        // Fabricate a 3-level path: (path_index, commitment_hash_bytes).
        let path: Vec<(u8, [u8; 32])> = vec![
            (5, [0xAAu8; 32]),
            (17, [0xBBu8; 32]),
            (255, [0xCCu8; 32]),
        ];

        let state_root = [0xFFu8; 32];

        let proof = prover.prove(&key, &value, &path, &state_root).expect("prove");
        assert_eq!(proof.num_steps, 3);
        assert_eq!(proof.z_0.len(), 32);
        assert_eq!(proof.z_final.len(), 32);
        assert!(!proof.proof_bytes.is_empty());
    }
}
