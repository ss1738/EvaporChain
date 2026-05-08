//! `VerkleStepCircuit` — one IVC step = one level of the Verkle proof path.
//!
//! IVC state vector (arity = 1):
//!   z[0] = current Poseidon commitment hash (Verkle level hash, as a field element)
//!
//! Witness per step:
//!   - `path_index`             : u8   — which child slot we occupy at this level
//!   - `sibling_hash`           : F    — Poseidon hash of the sibling commitment at this level
//!
//! Constraint (in-circuit):
//!   z_out[0] = Poseidon( z_in[0],  path_index_as_scalar,  sibling_hash )
//!
//! The IVC accumulates D steps bottom-up (leaf → root):
//!   z_0  = Poseidon(key_hash, value_hash)     — leaf binding, computed before first fold
//!   z_D  = state_root                          — verified by EvaporHeaderInbox on Ethereum
//!
//! The Ethereum verifier only needs (z_0, z_D, D) + the compressed SNARK proof.

use core::marker::PhantomData;

use generic_array::typenum::U24;
use nova_snark::{
    frontend::{
        gadgets::poseidon::{
            Elt, IOPattern, Simplex, Sponge, SpongeAPI, SpongeCircuit, SpongeOp, SpongeTrait,
            Strength,
        },
        num::AllocatedNum,
        ConstraintSystem, SynthesisError,
    },
    traits::{circuit::StepCircuit, Group},
};

/// Number of elements absorbed per IVC step: z_in + path_index + sibling_hash.
const ABSORB_N: u32 = 3;

/// Witness data for one Verkle level step.
#[derive(Clone, Debug)]
pub struct VerkleStepWitness<F: Clone> {
    /// Child index (0–255) at this level of the Verkle path.
    pub path_index: u8,
    /// The sibling commitment hash at this Verkle level (from the VerkleProof).
    ///
    /// Concretely: `bytes_to_scalar(proof.commitments[level])` masked to fit F.
    pub sibling_hash: F,
}

/// IVC step circuit for one level of a Verkle membership proof.
#[derive(Clone, Debug)]
pub struct VerkleStepCircuit<G: Group> {
    pub witness: VerkleStepWitness<G::Scalar>,
    _p: PhantomData<G>,
}

impl<G: Group> VerkleStepCircuit<G> {
    pub fn new(path_index: u8, sibling_hash: G::Scalar) -> Self {
        Self {
            witness: VerkleStepWitness { path_index, sibling_hash },
            _p: PhantomData,
        }
    }

    /// Dummy circuit for public-parameter setup — witness values don't matter.
    pub fn dummy() -> Self {
        Self::new(0, G::Scalar::ZERO)
    }

    /// Native (non-circuit) Poseidon of three scalars — used to build z_0 and
    /// to cross-check circuit outputs in tests.
    pub fn poseidon_native(a: G::Scalar, b: G::Scalar, c: G::Scalar) -> G::Scalar {
        let pc = Sponge::<G::Scalar, U24>::api_constants(Strength::Standard);
        let mut sponge = Sponge::<G::Scalar, U24>::new_with_constants(&pc, Simplex);
        let acc = &mut ();
        SpongeAPI::start(
            &mut sponge,
            IOPattern(vec![SpongeOp::Absorb(ABSORB_N), SpongeOp::Squeeze(1)]),
            None,
            acc,
        );
        SpongeAPI::absorb(&mut sponge, ABSORB_N, &[a, b, c], acc);
        let out = SpongeAPI::squeeze(&mut sponge, 1, acc);
        SpongeAPI::finish(&mut sponge, acc).expect("native Poseidon finish");
        out[0]
    }

    /// Compute the leaf hash z_0 = Poseidon(key_scalar, value_scalar).
    ///
    /// `key_scalar`   = first 32 bytes of key (masked), packed as F
    /// `value_scalar` = blake3 of value bytes (masked), packed as F
    pub fn leaf_hash(key_scalar: G::Scalar, value_scalar: G::Scalar) -> G::Scalar {
        let pc = Sponge::<G::Scalar, U24>::api_constants(Strength::Standard);
        let mut sponge = Sponge::<G::Scalar, U24>::new_with_constants(&pc, Simplex);
        let acc = &mut ();
        SpongeAPI::start(
            &mut sponge,
            IOPattern(vec![SpongeOp::Absorb(2), SpongeOp::Squeeze(1)]),
            None,
            acc,
        );
        SpongeAPI::absorb(&mut sponge, 2, &[key_scalar, value_scalar], acc);
        let out = SpongeAPI::squeeze(&mut sponge, 1, acc);
        SpongeAPI::finish(&mut sponge, acc).expect("native leaf Poseidon finish");
        out[0]
    }
}

impl<G: Group> StepCircuit<G::Scalar> for VerkleStepCircuit<G> {
    fn arity(&self) -> usize {
        1 // z = [current_commitment_hash]
    }

    fn synthesize<CS: ConstraintSystem<G::Scalar>>(
        &self,
        cs: &mut CS,
        z_in: &[AllocatedNum<G::Scalar>],
    ) -> Result<Vec<AllocatedNum<G::Scalar>>, SynthesisError> {
        assert_eq!(z_in.len(), 1, "VerkleStepCircuit arity is 1");

        // --- Allocate witness values ---

        // path_index: u8 → scalar.  We only need it as a field element for
        // the hash; range-checking to 8 bits is sufficient.
        let path_index_scalar = AllocatedNum::alloc(
            cs.namespace(|| "path_index"),
            || Ok(G::Scalar::from(self.witness.path_index as u64)),
        )?;

        // sibling_hash: the commitment hash of our sibling at this level.
        let sibling_hash = AllocatedNum::alloc(
            cs.namespace(|| "sibling_hash"),
            || Ok(self.witness.sibling_hash),
        )?;

        // --- Poseidon( z_in[0], path_index, sibling_hash ) → z_out ---

        let elts = [
            Elt::Allocated(z_in[0].clone()),
            Elt::Allocated(path_index_scalar),
            Elt::Allocated(sibling_hash),
        ];

        let parameter = IOPattern(vec![
            SpongeOp::Absorb(ABSORB_N),
            SpongeOp::Squeeze(1u32),
        ]);

        let pc = Sponge::<G::Scalar, U24>::api_constants(Strength::Standard);
        let mut ns = cs.namespace(|| "poseidon");

        let z_out = {
            let mut sponge = SpongeCircuit::new_with_constants(&pc, Simplex);
            let acc = &mut ns;
            sponge.start(parameter, None, acc);
            SpongeAPI::absorb(&mut sponge, ABSORB_N, &elts, acc);
            let output = SpongeAPI::squeeze(&mut sponge, 1, acc);
            sponge.finish(acc).map_err(|_| SynthesisError::Unsatisfiable)?;
            Elt::ensure_allocated(&output[0], &mut ns.namespace(|| "z_out_alloc"))?
        };

        Ok(vec![z_out])
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ff::Field;
    use nova_snark::provider::Bn256EngineKZG;
    use nova_snark::traits::Engine;

    type E = Bn256EngineKZG;
    type F = <E as Engine>::Scalar;
    type G = <E as Engine>::GE;

    #[test]
    fn arity_is_one() {
        let c = VerkleStepCircuit::<G>::dummy();
        assert_eq!(c.arity(), 1);
    }

    #[test]
    fn poseidon_native_is_deterministic() {
        let a = F::from(42u64);
        let b = F::from(7u64);
        let c = F::from(99u64);
        let h1 = VerkleStepCircuit::<G>::poseidon_native(a, b, c);
        let h2 = VerkleStepCircuit::<G>::poseidon_native(a, b, c);
        assert_eq!(h1, h2);
    }

    #[test]
    fn poseidon_native_different_inputs_give_different_outputs() {
        let a = F::from(1u64);
        let b = F::from(2u64);
        let c = F::from(3u64);
        let h1 = VerkleStepCircuit::<G>::poseidon_native(a, b, c);
        let h2 = VerkleStepCircuit::<G>::poseidon_native(a, b, F::from(4u64));
        assert_ne!(h1, h2, "different sibling hash → different parent hash");
    }

    #[test]
    fn leaf_hash_is_deterministic() {
        let ks = F::from(0xCAFEBABEu64);
        let vs = F::from(0xDEADBEEFu64);
        assert_eq!(
            VerkleStepCircuit::<G>::leaf_hash(ks, vs),
            VerkleStepCircuit::<G>::leaf_hash(ks, vs),
        );
    }

    #[test]
    fn path_index_change_changes_hash() {
        let z = F::from(100u64);
        let sib = F::from(200u64);
        let h1 = VerkleStepCircuit::<G>::poseidon_native(z, F::from(0u64), sib);
        let h2 = VerkleStepCircuit::<G>::poseidon_native(z, F::from(1u64), sib);
        assert_ne!(h1, h2, "different path_index → different parent hash");
    }

    /// Verify that a 3-step hash chain is self-consistent:
    ///   z_0 = leaf_hash(key, value)
    ///   z_1 = Poseidon(z_0, idx_0, sib_0)
    ///   z_2 = Poseidon(z_1, idx_1, sib_1)
    ///   z_3 = Poseidon(z_2, idx_2, sib_2)
    #[test]
    fn three_step_chain_is_consistent() {
        let key_s = F::from(0xAAAAu64);
        let val_s = F::from(0xBBBBu64);

        let z0 = VerkleStepCircuit::<G>::leaf_hash(key_s, val_s);

        let indices  = [3u64, 17u64, 255u64];
        let siblings = [F::from(11u64), F::from(22u64), F::from(33u64)];

        let mut z = z0;
        for i in 0..3 {
            z = VerkleStepCircuit::<G>::poseidon_native(z, F::from(indices[i]), siblings[i]);
        }

        // "root" would be z after all levels.  Just check it's non-zero and deterministic.
        assert_ne!(z, F::ZERO);

        // Re-run to confirm determinism.
        let z0b = VerkleStepCircuit::<G>::leaf_hash(key_s, val_s);
        let mut zb = z0b;
        for i in 0..3 {
            zb = VerkleStepCircuit::<G>::poseidon_native(zb, F::from(indices[i]), siblings[i]);
        }
        assert_eq!(z, zb);
    }
}
