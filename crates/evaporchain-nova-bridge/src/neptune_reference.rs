//! Neptune-Poseidon ground-truth oracle.
//!
//! Wraps nova-snark's public [`PoseidonRO`] API (which uses the same
//! neptune Poseidon implementation `RecursiveSNARK::verify` consumes
//! internally) into a single function `neptune_hash_primary`. Pinned
//! test vectors in this module are the byte-level reference for any
//! Section-2 hash output port.
//!
//! # Role within the constants-port chain
//!
//! - **Constants layer**: byte-correct vs neptune
//!   (`crate::grain_lfsr` + `crate::compress_ark`, PR #103).
//! - **Hash output**: still diverges from this oracle's output
//!   because arkworks `PoseidonSpongeVar`'s per-round operation
//!   differs from neptune's `Poseidon::hash_optimized_static`
//!   (the residual sponge framing gap). See
//!   `crate::section2_gadget::fully_aligned_gadget_byte_parity_with_neptune`.
//!
//! # Why pin specific values
//!
//! Hardcoded test vectors are the only way to detect a silent
//! constant drift between this oracle and a future arkworks gadget.
//! If both sides change in the same wrong way, no test catches it.
//! Pinning fixed scalars means: if THIS module's output changes
//! (e.g. nova-snark bumps its neptune fork), the test fires loudly
//! and forces a regen of test vectors that downstream gadgets need
//! to match.

use nova_snark::{
    provider::poseidon::{PoseidonConstantsCircuit, PoseidonRO},
    traits::ROTrait,
};

/// Type alias for the primary-side scalar field nova uses with
/// `Bn256EngineKZG`. Same field as `ark_bn254::Fr`; conversion via
/// the (PR-#66) `scalar_adapter`.
pub type PrimaryScalar = nova_snark::provider::bn256_grumpkin::bn256::Scalar;

/// Run an absorb→squeeze cycle on neptune Poseidon over the BN254
/// scalar field, returning the squeezed scalar.
///
/// Mirrors the operation nova-snark's `PoseidonRO::squeeze` does
/// inside `RecursiveSNARK::verify` to produce `hash_primary`.
/// `start_with_one` matches the verifier call (set to `false`).
pub fn neptune_hash_primary(absorbed: &[PrimaryScalar]) -> PrimaryScalar {
    let constants = PoseidonConstantsCircuit::<PrimaryScalar>::default();
    let mut sponge = PoseidonRO::<PrimaryScalar>::new(constants);
    for s in absorbed {
        sponge.absorb(*s);
    }
    sponge.squeeze(crate::neptune_reference::NUM_HASH_BITS, false)
}

/// Truncation parameter nova-snark uses on every transcript squeeze.
///
/// Matches `nova-snark::constants::NUM_HASH_BITS = 250`. Documenting
/// here so the test pin doesn't accidentally drift if a future
/// nova-snark release changes it.
pub const NUM_HASH_BITS: usize = 250;

#[cfg(test)]
mod tests {
    use super::*;
    use ff::{Field, PrimeField};

    /// Reproducibility: a fixed absorb sequence must always produce
    /// the same scalar across runs. If this ever fires, a non-
    /// deterministic neptune setting has leaked in (or constants
    /// shifted).
    #[test]
    fn hash_is_deterministic() {
        let inputs = vec![
            PrimaryScalar::from(1u64),
            PrimaryScalar::from(2u64),
            PrimaryScalar::from(3u64),
        ];
        let a = neptune_hash_primary(&inputs);
        let b = neptune_hash_primary(&inputs);
        assert_eq!(a, b, "neptune hash must be deterministic for fixed input");
    }

    /// Distinguishability: different inputs must produce different
    /// hashes. Catches a hypothetical neptune-collision-by-accident
    /// caused by misconfigured parameters.
    #[test]
    fn distinct_inputs_distinct_hashes() {
        let h1 = neptune_hash_primary(&[PrimaryScalar::from(1u64)]);
        let h2 = neptune_hash_primary(&[PrimaryScalar::from(2u64)]);
        assert_ne!(h1, h2, "distinct inputs must hash distinctly");

        let h_seq_a = neptune_hash_primary(&[
            PrimaryScalar::from(1u64),
            PrimaryScalar::from(2u64),
        ]);
        let h_seq_b = neptune_hash_primary(&[
            PrimaryScalar::from(2u64),
            PrimaryScalar::from(1u64),
        ]);
        assert_ne!(
            h_seq_a, h_seq_b,
            "order must matter (sponge is not commutative)"
        );
    }

    /// Empty input edge case: an empty absorb sequence still
    /// produces a well-defined hash (sponge initial state +
    /// permutation). Lock down the behaviour so the arkworks gadget
    /// has a clear edge-case expectation.
    #[test]
    fn empty_input_produces_well_defined_hash() {
        let h = neptune_hash_primary(&[]);
        // The value itself is not what we care about for now —
        // just that the call doesn't panic and returns a valid
        // scalar (the squeeze converts to scalar form).
        // Document the empty hash for future arkworks-side parity.
        eprintln!("neptune_hash_primary([]) = {h:?}");
        // Re-call to confirm determinism.
        assert_eq!(h, neptune_hash_primary(&[]));
    }

    /// Pin the BN254 scalar zero/one constants resolve through the
    /// scalar adapter to match (smoke test that nova's halo2curves
    /// `bn256::Scalar` field arithmetic is what we expect). This
    /// rules out a wrong-field-type integration in case
    /// `Bn256EngineKZG` ever rewires its scalar field.
    #[test]
    fn nova_primary_scalar_field_basics() {
        assert_eq!(PrimaryScalar::ZERO, PrimaryScalar::from(0u64));
        assert_eq!(PrimaryScalar::ONE, PrimaryScalar::from(1u64));
        let two = PrimaryScalar::ONE + PrimaryScalar::ONE;
        assert_eq!(two, PrimaryScalar::from(2u64));
    }

    /// Reference vector for the arkworks port to match: `[42, 7, 99]`.
    /// Bytes captured from a prior Mini-1 run of this test and locked
    /// in here as `[u8; 32]` little-endian (the `to_repr()` form).
    ///
    /// When Section 2's arkworks gadget lands, it absorbs the same
    /// sequence and must `.to_repr() == EXPECTED`. If THIS test ever
    /// fires, nova-snark's neptune fork has changed its parameters
    /// and the arkworks port needs re-verifying.
    #[test]
    fn pinned_reference_vector_42_7_99() {
        let inputs = vec![
            PrimaryScalar::from(42u64),
            PrimaryScalar::from(7u64),
            PrimaryScalar::from(99u64),
        ];
        let h = neptune_hash_primary(&inputs);
        const EXPECTED_LE: [u8; 32] = [
            131, 47, 215, 132, 108, 127, 155, 157, 242, 38, 202, 31, 128, 130, 76, 218, 237, 255,
            134, 244, 210, 204, 231, 79, 221, 11, 190, 164, 69, 163, 134, 1,
        ];
        let repr: [u8; 32] = h.to_repr().into();
        assert_eq!(
            repr, EXPECTED_LE,
            "neptune_hash_primary([42, 7, 99]) drifted from pinned reference"
        );
    }

    /// Reference vector for Section 2's PRIMARY-side absorb sequence
    /// at `z_arity = 1` with placeholder values:
    ///   `[pp.digest=0, num_steps=1, z0[0]=0, zi[0]=1, ri_primary=0]`
    ///
    /// This is the *exact shape* (but with zero / trivial values) of
    /// what nova-snark's `RecursiveSNARK::verify` absorbs to produce
    /// `hash_primary` — minus the cross-side `r_U_secondary` instance
    /// absorb which expands to multiple scalars at adapter time. The
    /// Section 2 gadget at `z_arity = 1` will absorb the same shape
    /// (with real values) and must produce the matching output for
    /// these placeholders.
    ///
    /// The byte vector is pinned from a prior Mini-1 run; if it ever
    /// changes, future Section 2 gadgets need re-verifying.
    #[test]
    fn pinned_section_2_primary_minimal_absorb_sequence() {
        let inputs = vec![
            PrimaryScalar::ZERO,         // pp.digest (placeholder)
            PrimaryScalar::from(1u64),   // num_steps
            PrimaryScalar::ZERO,         // z0[0]
            PrimaryScalar::from(1u64),   // zi[0]
            PrimaryScalar::ZERO,         // ri_primary
        ];
        let h = neptune_hash_primary(&inputs);
        const EXPECTED_LE: [u8; 32] = [
            119, 216, 133, 86, 248, 52, 238, 11, 170, 80, 203, 180, 56, 189, 50, 158, 37, 224, 8,
            145, 66, 66, 174, 238, 128, 144, 18, 209, 173, 117, 250, 2,
        ];
        let repr: [u8; 32] = h.to_repr().into();
        assert_eq!(
            repr, EXPECTED_LE,
            "Section-2 primary minimal absorb output drifted from pinned reference"
        );
    }
}
