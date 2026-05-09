//! `EccVerkleStepCircuit` — V2 of the per-level Verkle IVC step.
//!
//! Lane T0.9 from `MAINNET_READINESS.md`. The full lane (~2-3 weeks)
//! replaces V1's Poseidon-hash binding (`crate::circuit::VerkleStepCircuit`)
//! with native EC MSM in-circuit using Halo2 `EccChip`. This file is
//! the **sub-task A scaffold** — types, feature gating, and the
//! planned wiring points are in place; the actual constraint
//! implementation lands across sub-tasks B–D.
//!
//! Why V2 matters: V1 binds (key, value, root) via a collision-
//! resistant hash chain (Poseidon), with the EC Pedersen-commitment
//! check offloaded to the prover's `VerkleProof::verify()`. V2 moves
//! the EC check INTO the circuit, so the verifier (on Ethereum)
//! validates the EC arithmetic that produced the claimed Verkle
//! root — strengthening the assumption from "the prover's verify
//! function ran honestly" to "the EC point arithmetic is constrained
//! by the Halo2 statement we publish."
//!
//! ## V1 vs V2 binding (per-step)
//!
//! V1 (this turn, shipped at `circuit.rs:118`):
//! ```text
//!   z_out[0] = Poseidon(z_in[0], path_index, sibling_hash)
//! ```
//!
//! V2 (lane T0.9 target):
//! ```text
//!   commitment = pedersen_commit(level_basis, witness_scalars)
//!   asserted_eq!(commitment, expected_commitment_at_level)
//!   z_out[0]   = bind_to_field(commitment)   // for IVC accumulator
//! ```
//!
//! The EC MSM (`pedersen_commit`) is the constraint added by
//! `halo2_gadgets::ecc::EccChip` — it forces the prover to perform
//! actual point addition + scalar multiplication, not just a hash
//! the verifier could fake.
//!
//! ## Sub-task layering inside T0.9
//!
//! | Sub-task | Effort | Surface |
//! |---|---|---|
//! | A — feature flag + skeleton (THIS COMMIT) | <1 day | `Cargo.toml` + `circuit_v2.rs` stub |
//! | B — `EccVerkleStepCircuit::synthesize` body w/ `EccChip` config | 5-7 days | `synthesize`, `Config`, fixed-base loading |
//! | C — Pasta `pallas` MSM gadget integration + native counterpart | 5-7 days | `pedersen_commit_native`, `pedersen_commit_circuit` parity |
//! | D — `VerkleProver::prove_v2` + cross-side fixture | 3-5 days | `prover.rs`, fixture JSON, end-to-end test |
//!
//! Sub-tasks B/C/D land as separate lane claims; the structural seam
//! introduced here is the contract those follow-ups commit against.
//!
//! ## Compile gating
//!
//! Off by default. The V1 path (`crate::circuit::VerkleStepCircuit`)
//! continues to be the prover's authoritative path until D wires V2
//! through. With `--features v2-ecc`, `halo2_proofs` and
//! `halo2_gadgets` build into the dep graph and this module's stub
//! compiles. Tests under `--features v2-ecc` only verify scaffold
//! shape; real circuit-correctness tests land in sub-task B.

#![cfg(feature = "v2-ecc")]

use ff::{Field, PrimeField};

/// Witness for one EccVerkleStepCircuit level. Same shape as
/// `VerkleStepWitness` but the validation path is EC-MSM-bound, not
/// Poseidon-bound.
///
/// Fields are the per-level witness: the sibling-commitment point's
/// coordinates (instead of just its Poseidon hash), the path index,
/// and the per-level basis scalars. Sub-task B fills out the actual
/// field set — this skeleton uses the V1 shape so the API is
/// recognisable.
#[derive(Clone, Debug)]
pub struct EccVerkleStepWitness<F: PrimeField + Clone> {
    /// Sibling-commitment x-coordinate (Pasta `pallas` base field).
    pub sibling_x: F,
    /// Sibling-commitment y-coordinate.
    pub sibling_y: F,
    /// Path index at this level (0 or 1 for binary, in `[0, k)` for
    /// k-ary). Same semantics as V1's `path_index`.
    pub path_index: F,
}

/// V2 step circuit — EC MSM-bound IVC step.
///
/// Type-parametrised over the curve's scalar field `F` so the same
/// scaffold serves Pasta `pallas` and Pasta `vesta` (the inner/outer
/// pair Halo2 uses for recursive proof composition).
///
/// **Sub-task A scaffold:** the struct shape + the trait stubs are
/// here; `synthesize` is a TODO that returns `Ok(())` so the type
/// compiles. Sub-task B replaces that with the EccChip config +
/// MSM constraint body.
#[derive(Clone, Debug)]
pub struct EccVerkleStepCircuit<F: PrimeField + Clone> {
    pub witness: EccVerkleStepWitness<F>,
}

impl<F: PrimeField + Clone> EccVerkleStepCircuit<F> {
    /// Construct a step circuit from witness data. Mirrors V1's
    /// `VerkleStepCircuit::new` shape.
    pub fn new(witness: EccVerkleStepWitness<F>) -> Self {
        Self { witness }
    }

    /// Construct a placeholder/dummy step circuit for prover setup
    /// (where the witness shape matters but the values are unused).
    /// Mirrors V1's `dummy()`.
    pub fn dummy() -> Self {
        Self {
            witness: EccVerkleStepWitness {
                sibling_x: F::ZERO,
                sibling_y: F::ZERO,
                path_index: F::ZERO,
            },
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
// Sub-task B placeholder — `impl Circuit<F>` block goes here.
//
// Shape (commented to keep this commit gate-clean):
//
// impl<F: PrimeField> halo2_proofs::plonk::Circuit<F> for EccVerkleStepCircuit<F> {
//     type Config = EccVerkleStepConfig;
//     type FloorPlanner = halo2_proofs::circuit::SimpleFloorPlanner;
//
//     fn without_witnesses(&self) -> Self { Self::dummy() }
//
//     fn configure(meta: &mut ConstraintSystem<F>) -> Self::Config {
//         // Allocate Halo2 columns; configure EccChip with
//         // halo2_gadgets::ecc::Config; declare custom gates for
//         // the level-binding relation.
//         todo!("sub-task B")
//     }
//
//     fn synthesize(
//         &self,
//         config: Self::Config,
//         layouter: impl Layouter<F>,
//     ) -> Result<(), halo2_proofs::plonk::Error> {
//         // 1. Load fixed-base bases for this Verkle level.
//         // 2. Witness the sibling commitment point + scalars.
//         // 3. Constrain pedersen_commit(bases, scalars) == sibling.
//         // 4. Bind level-output to field for the IVC accumulator.
//         todo!("sub-task B")
//     }
// }
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ff::Field;
    use pasta_curves::Fp;

    #[test]
    fn ecc_step_witness_constructs() {
        let w = EccVerkleStepWitness::<Fp> {
            sibling_x: Fp::from(1u64),
            sibling_y: Fp::from(2u64),
            path_index: Fp::from(0u64),
        };
        assert_eq!(w.path_index, Fp::from(0u64));
    }

    #[test]
    fn ecc_step_circuit_dummy_constructs() {
        let c = EccVerkleStepCircuit::<Fp>::dummy();
        assert_eq!(c.witness.sibling_x, Fp::ZERO);
        assert_eq!(c.witness.sibling_y, Fp::ZERO);
        assert_eq!(c.witness.path_index, Fp::ZERO);
    }

    #[test]
    fn ecc_step_circuit_new_preserves_witness() {
        let w = EccVerkleStepWitness::<Fp> {
            sibling_x: Fp::from(7u64),
            sibling_y: Fp::from(11u64),
            path_index: Fp::from(13u64),
        };
        let c = EccVerkleStepCircuit::<Fp>::new(w.clone());
        assert_eq!(c.witness.sibling_x, w.sibling_x);
        assert_eq!(c.witness.sibling_y, w.sibling_y);
        assert_eq!(c.witness.path_index, w.path_index);
    }
}
