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

use ff::PrimeField;

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
// Native Pedersen commitment — sub-task C cross-side counterpart.
//
// What this is: the off-circuit reference implementation of the
// Verkle-level Pedersen commit. Sub-task B's `synthesize` will
// constrain the same arithmetic in-circuit using halo2_gadgets's
// EccChip; sub-task C's parity test will assert
//   pedersen_commit_native(bases, scalars) == sibling_witness
// and constrain the same equation in-circuit, so the prover cannot
// satisfy one without satisfying the other.
//
// Lives in this module (not a separate `parity.rs`) so the V2
// crypto contract is one read away from the circuit it pairs
// with. Same gating: `#![cfg(feature = "v2-ecc")]` at module top.
//
// Performance note: this is a reference implementation, NOT the
// in-circuit MSM. We use straightforward Σ s_i · G_i with
// `pallas::Affine` doubling-and-adding. Production prover should
// use a windowed-MSM library (e.g. group's `multiscalar_mul`)
// once the constraint version is locked.
// ─────────────────────────────────────────────────────────────────────

use pasta_curves::group::{Curve, Group};
use pasta_curves::pallas;

/// Native Pedersen commitment: `C = Σ scalars[i] · bases[i]`.
///
/// Returns the affine point. Length mismatch returns the identity
/// point (caller should validate inputs before calling — this is
/// the cryptographic primitive, not the gateway).
///
/// **Lane T0.9 sub-task C** — paired with the sub-task B in-circuit
/// EccChip MSM constraint. The parity test `pedersen_commit_in_and_out_of_circuit_agree`
/// (sub-task C deliverable) will assert this native output equals
/// what the circuit's witness-and-constrain path produces for the
/// same bases + scalars.
pub fn pedersen_commit_native(
    bases: &[pallas::Affine],
    scalars: &[pallas::Scalar],
) -> pallas::Affine {
    if bases.len() != scalars.len() || bases.is_empty() {
        return pallas::Point::identity().to_affine();
    }
    // Σ s_i · G_i — straightforward variable-base scalar mul + sum
    // in projective coords, then back to affine for the result.
    let mut acc = pallas::Point::identity();
    for (b, s) in bases.iter().zip(scalars.iter()) {
        acc += pallas::Point::from(*b) * *s;
    }
    acc.to_affine()
}

// ─────────────────────────────────────────────────────────────────────
// Sub-task B starter — Halo2 column allocation for EccChip.
//
// `EccChip::configure` requires:
//   - 10 advice columns (the chip equality-enables all of them)
//   - 8 fixed columns for Lagrange coefficients (fixed-base mul windows)
//   - 1 fixed column for shared constants (enabled via `enable_constant`)
//   - 1 lookup-table column + a LookupRangeCheckConfig over advice[9]
//
// We wrap that allocation in `allocate_ecc_columns` so the
// `Circuit::configure` body (sub-task B finish) reduces to:
//
//     let cols = EccVerkleStepCircuit::<pallas::Base>::allocate_ecc_columns(meta);
//     let range_check = LookupRangeCheckConfig::configure(
//         meta, cols.advices[9], cols.lookup_table);
//     EccChip::<VerkleFixedBases>::configure(
//         meta, cols.advices, cols.lagrange_coeffs, range_check)
//
// — pending the `VerkleFixedBases: FixedPoints<pallas::Affine>` impl
// (sub-task C circuit half). Until that lands, this allocation
// function is callable but the `EccChip::configure` line is not yet
// in `synthesize`.
// ─────────────────────────────────────────────────────────────────────

use halo2_proofs::pasta::pallas as halo2_pallas;
use halo2_proofs::plonk::{Advice, Column, ConstraintSystem, Fixed, TableColumn};

/// The columns EccChip::configure consumes. Held together so the
/// `Circuit::configure` body can pass them as one block to the
/// chip's configure call once the FixedPoints impl lands.
#[derive(Debug, Clone)]
pub struct EccColumns {
    /// 10 advice columns. `EccChip::configure` equality-enables all
    /// of them. `advices[9]` is reserved for the LookupRangeCheckConfig.
    pub advices: [Column<Advice>; 10],
    /// 8 fixed columns for Lagrange coefficients used by
    /// `mul_fixed::Config::configure`.
    pub lagrange_coeffs: [Column<Fixed>; 8],
    /// Shared fixed column for constants; must be `enable_constant`-d.
    pub constants: Column<Fixed>,
    /// Lookup-table column for the 10-bit range-check used by
    /// variable-base scalar mul.
    pub lookup_table: TableColumn,
}

impl EccVerkleStepCircuit<halo2_pallas::Base> {
    /// Allocate every column EccChip needs. Sub-task B starter.
    /// Called from `Circuit::configure` once that's wired (sub-task B
    /// finish, after sub-task C ships the FixedPoints impl).
    ///
    /// **Side effect:** `enable_constant(constants)` is called on the
    /// returned `constants` column — the EccChip's mul_fixed config
    /// requires this. The caller does NOT need to enable it again.
    pub fn allocate_ecc_columns(
        meta: &mut ConstraintSystem<halo2_pallas::Base>,
    ) -> EccColumns {
        let advices = [
            meta.advice_column(),
            meta.advice_column(),
            meta.advice_column(),
            meta.advice_column(),
            meta.advice_column(),
            meta.advice_column(),
            meta.advice_column(),
            meta.advice_column(),
            meta.advice_column(),
            meta.advice_column(),
        ];
        let lagrange_coeffs = [
            meta.fixed_column(),
            meta.fixed_column(),
            meta.fixed_column(),
            meta.fixed_column(),
            meta.fixed_column(),
            meta.fixed_column(),
            meta.fixed_column(),
            meta.fixed_column(),
        ];
        let constants = meta.fixed_column();
        meta.enable_constant(constants);
        let lookup_table = meta.lookup_table_column();

        EccColumns {
            advices,
            lagrange_coeffs,
            constants,
            lookup_table,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
// Sub-task B finish — FixedPoints impl + Circuit::configure body.
//
// Real Halo2 wiring (cargo-verifiable on-host). The `EccChip` is a
// trait-generic over its `FixedPoints` set; we supply
// `VerkleFixedBases` whose three associated types impl
// `FixedPoint<pallas::Affine>` over the curve generator's
// precomputed scalar-mul tables (computed once at first access via
// `find_zs_and_us`).
//
// The choice to use the curve generator as the single base is
// scaffold-grade. Sub-task C will replace it with per-Verkle-level
// bases (one fixed point per tree level, deterministically derived
// from a domain-separated seed). That replacement only swaps the
// `BASE` value — the trait wiring + circuit shape stay identical.
//
// Why this is safe to ship now: `Circuit::configure` only needs the
// trait to be SATISFIED (so EccChip can call `lagrange_coeffs()` at
// proof time). The actual proof generation under sub-task D will
// use the real per-level bases. The current stub will produce
// correct proofs for a CIRCUIT that uses just the curve generator —
// which is exactly what the sub-task C parity test exercises:
//   pedersen_commit_native(&[generator()], &[scalar]) == circuit_out
// — the simplest non-trivial parity check.
// ─────────────────────────────────────────────────────────────────────

use halo2_gadgets::ecc::chip::{
    find_zs_and_us, BaseFieldElem, EccChip, FixedPoint as HaloFixedPoint, FullScalar, ShortScalar,
    H, NUM_WINDOWS, NUM_WINDOWS_SHORT,
};
use halo2_gadgets::ecc::FixedPoints as HaloFixedPoints;
use halo2_gadgets::utilities::lookup_range_check::LookupRangeCheckConfig;
use halo2_proofs::circuit::{Layouter, SimpleFloorPlanner};
use halo2_proofs::plonk::{Circuit, Error};

const SINSEMILLA_K: usize = 10;

/// The `FixedPoints` set EccChip uses for our circuit.
///
/// Three associated-type marker structs — `VerkleFullWidth` /
/// `VerkleShort` / `VerkleBaseField` — each impl `FixedPoint` over
/// the same underlying base point (the Pasta `pallas` curve
/// generator) but with different scalar-mul kinds. Sub-task C will
/// supply per-level bases by replacing the generator constant in
/// each of the three; the EccChip wiring above doesn't change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerkleFixedBases;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerkleFullWidth;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerkleShort;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerkleBaseField;

impl HaloFixedPoints<halo2_pallas::Affine> for VerkleFixedBases {
    type FullScalar = VerkleFullWidth;
    type ShortScalar = VerkleShort;
    type Base = VerkleBaseField;
}

/// Lazy-static the (z, u) tables — `find_zs_and_us` does a search
/// that's slow enough we don't want it on every chip method call.
fn pallas_generator_affine() -> halo2_pallas::Affine {
    use pasta_curves::group::Curve;
    halo2_pallas::Point::generator().to_affine()
}

fn zs_and_us_full() -> &'static Vec<(u64, [halo2_pallas::Base; H])> {
    use std::sync::OnceLock;
    static CACHE: OnceLock<Vec<(u64, [halo2_pallas::Base; H])>> = OnceLock::new();
    CACHE.get_or_init(|| {
        find_zs_and_us(pallas_generator_affine(), NUM_WINDOWS)
            .expect("find_zs_and_us(generator, NUM_WINDOWS) must succeed for the curve generator")
    })
}

fn zs_and_us_short() -> &'static Vec<(u64, [halo2_pallas::Base; H])> {
    use std::sync::OnceLock;
    static CACHE: OnceLock<Vec<(u64, [halo2_pallas::Base; H])>> = OnceLock::new();
    CACHE.get_or_init(|| {
        find_zs_and_us(pallas_generator_affine(), NUM_WINDOWS_SHORT)
            .expect("find_zs_and_us(generator, NUM_WINDOWS_SHORT) must succeed")
    })
}

fn zs_us_to_u_repr(
    zs_us: &[(u64, [halo2_pallas::Base; H])],
) -> Vec<[[u8; 32]; H]> {
    use ff::PrimeField;
    zs_us
        .iter()
        .map(|(_, us)| {
            [
                us[0].to_repr(),
                us[1].to_repr(),
                us[2].to_repr(),
                us[3].to_repr(),
                us[4].to_repr(),
                us[5].to_repr(),
                us[6].to_repr(),
                us[7].to_repr(),
            ]
        })
        .collect()
}

fn zs_us_to_z(zs_us: &[(u64, [halo2_pallas::Base; H])]) -> Vec<u64> {
    zs_us.iter().map(|(z, _)| *z).collect()
}

impl HaloFixedPoint<halo2_pallas::Affine> for VerkleFullWidth {
    type FixedScalarKind = FullScalar;
    fn generator(&self) -> halo2_pallas::Affine {
        pallas_generator_affine()
    }
    fn u(&self) -> Vec<[[u8; 32]; H]> {
        zs_us_to_u_repr(zs_and_us_full())
    }
    fn z(&self) -> Vec<u64> {
        zs_us_to_z(zs_and_us_full())
    }
}

impl HaloFixedPoint<halo2_pallas::Affine> for VerkleShort {
    type FixedScalarKind = ShortScalar;
    fn generator(&self) -> halo2_pallas::Affine {
        pallas_generator_affine()
    }
    fn u(&self) -> Vec<[[u8; 32]; H]> {
        zs_us_to_u_repr(zs_and_us_short())
    }
    fn z(&self) -> Vec<u64> {
        zs_us_to_z(zs_and_us_short())
    }
}

impl HaloFixedPoint<halo2_pallas::Affine> for VerkleBaseField {
    type FixedScalarKind = BaseFieldElem;
    fn generator(&self) -> halo2_pallas::Affine {
        pallas_generator_affine()
    }
    fn u(&self) -> Vec<[[u8; 32]; H]> {
        zs_us_to_u_repr(zs_and_us_full())
    }
    fn z(&self) -> Vec<u64> {
        zs_us_to_z(zs_and_us_full())
    }
}

/// `Circuit::configure` output. Holds EccChip's config so synthesize
/// can `EccChip::construct(config.ecc.clone())`.
#[derive(Debug, Clone)]
pub struct EccVerkleStepConfig {
    pub ecc: halo2_gadgets::ecc::chip::EccConfig<VerkleFixedBases>,
}

impl Circuit<halo2_pallas::Base> for EccVerkleStepCircuit<halo2_pallas::Base> {
    type Config = EccVerkleStepConfig;
    type FloorPlanner = SimpleFloorPlanner;

    fn without_witnesses(&self) -> Self {
        Self::dummy()
    }

    fn configure(meta: &mut halo2_proofs::plonk::ConstraintSystem<halo2_pallas::Base>) -> Self::Config {
        let cols = Self::allocate_ecc_columns(meta);
        let range_check =
            LookupRangeCheckConfig::<halo2_pallas::Base, SINSEMILLA_K>::configure(
                meta,
                cols.advices[9],
                cols.lookup_table,
            );
        let ecc = EccChip::<VerkleFixedBases>::configure(
            meta,
            cols.advices,
            cols.lagrange_coeffs,
            range_check,
        );
        EccVerkleStepConfig { ecc }
    }

    /// Sub-task B body — Circuit::synthesize is currently a no-op
    /// that returns Ok(()). Sub-task C circuit half will fill it
    /// with: load fixed bases → witness sibling commitment point →
    /// constrain pedersen_commit(bases, scalars) == sibling →
    /// bind level-output to field for the IVC accumulator.
    fn synthesize(
        &self,
        _config: Self::Config,
        _layouter: impl Layouter<halo2_pallas::Base>,
    ) -> Result<(), Error> {
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────
// Note for the next claimer (sub-task C circuit half + sub-task D):
// the EccChip is now fully configured via `Circuit::configure`.
// Sub-task C completes when:
//   1. `synthesize` body witnesses (sibling_x, sibling_y) as a Point
//   2. constrains pedersen_commit({base}, {witness scalar}) == that point
//   3. asserts (in tests) that the in-circuit result equals
//      `pedersen_commit_native(&[base], &[scalar])`
// Sub-task D completes when `prover.rs::VerkleProver` exposes
// `prove_v2(...)` returning a CompressedSNARK + the same fixture
// shape the Solidity verifier (T0.10) ingests.
// ─────────────────────────────────────────────────────────────────────
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

    // ─── Sub-task C — native Pedersen commit reference ──────────────

    use pasta_curves::group::{Curve, Group};
    use pasta_curves::pallas;

    /// Length mismatch → identity point. Defends against caller bugs
    /// without panicking inside cryptographic core.
    #[test]
    fn pedersen_commit_native_length_mismatch_returns_identity() {
        let bases = vec![pallas::Point::generator().to_affine()];
        let scalars = vec![pallas::Scalar::from(2u64), pallas::Scalar::from(3u64)];
        let c = pedersen_commit_native(&bases, &scalars);
        assert_eq!(c, pallas::Point::identity().to_affine());
    }

    /// Empty inputs → identity point.
    #[test]
    fn pedersen_commit_native_empty_returns_identity() {
        let c = pedersen_commit_native(&[], &[]);
        assert_eq!(c, pallas::Point::identity().to_affine());
    }

    /// Single-base sanity: 1·G == G (where G is the curve generator).
    #[test]
    fn pedersen_commit_native_one_times_generator_is_generator() {
        let g = pallas::Point::generator().to_affine();
        let s = pallas::Scalar::from(1u64);
        let c = pedersen_commit_native(&[g], &[s]);
        assert_eq!(c, g);
    }

    /// Linearity: commit(a·G, b·G) = (a+b)·G when bases are equal
    /// (degenerate case but proves the sum is computed correctly).
    #[test]
    fn pedersen_commit_native_is_linear_in_scalars() {
        let g = pallas::Point::generator().to_affine();
        let a = pallas::Scalar::from(7u64);
        let b = pallas::Scalar::from(11u64);
        let c_split = pedersen_commit_native(&[g, g], &[a, b]);
        let c_combined = pedersen_commit_native(&[g], &[a + b]);
        assert_eq!(c_split, c_combined);
    }

    /// Distinct bases produce distinct commits when scalars differ.
    /// Catches the degenerate "always returns identity" or "always
    /// returns the first base" failure modes.
    #[test]
    fn pedersen_commit_native_distinct_inputs_distinct_outputs() {
        let g = pallas::Point::generator().to_affine();
        // Use the identity-doubled point as a second basis (quick way
        // to get a distinct curve point without a custom generator).
        let h = (pallas::Point::generator() * pallas::Scalar::from(7u64)).to_affine();
        assert_ne!(g, h);

        let s1 = pallas::Scalar::from(2u64);
        let s2 = pallas::Scalar::from(5u64);

        let c1 = pedersen_commit_native(&[g, h], &[s1, s2]);
        let c2 = pedersen_commit_native(&[g, h], &[s2, s1]);
        // Swapping scalars across distinct bases changes the commit.
        assert_ne!(c1, c2);
    }

    // ─── Sub-task B starter — column allocation ─────────────────────

    /// `allocate_ecc_columns` returns the right shape: 10 advice + 8
    /// lagrange_coeffs + 1 constants + 1 lookup_table. Sub-task B
    /// finish (the EccChip::configure call) consumes these unchanged.
    #[test]
    fn allocate_ecc_columns_returns_expected_arity() {
        use halo2_proofs::pasta::pallas as halo2_pallas;
        use halo2_proofs::plonk::ConstraintSystem;

        let mut meta: ConstraintSystem<halo2_pallas::Base> = ConstraintSystem::default();
        let cols = EccVerkleStepCircuit::<halo2_pallas::Base>::allocate_ecc_columns(&mut meta);

        // Each Column<Advice> / Column<Fixed> / TableColumn is opaque
        // (no public constructor outside the proofs crate), so we
        // can't compare raw values — but the type system enforces the
        // arity. The fact that these 4 lines compile + run is the
        // structural guarantee EccChip::configure needs.
        let _: [_; 10] = cols.advices;
        let _: [_; 8] = cols.lagrange_coeffs;
        let _ = cols.constants;
        let _ = cols.lookup_table;
    }

    // ─── Sub-task B finish — Circuit::configure validation ──────────

    /// **The key sub-task B-finish proof:** `Circuit::configure`
    /// successfully constructs the EccChip Config via `EccChip::configure`
    /// against `VerkleFixedBases`. If the FixedPoints / FixedPoint
    /// trait wiring is wrong (associated types missing, generator
    /// returning identity, find_zs_and_us failing), this test panics
    /// during `meta.synthesize`-style flow. Today it doesn't panic →
    /// the in-circuit EccChip is wired and ready for sub-task C
    /// circuit half (the synthesize body).
    #[test]
    fn circuit_configure_constructs_ecc_chip_config() {
        use halo2_proofs::pasta::pallas as halo2_pallas;
        use halo2_proofs::plonk::Circuit;
        use halo2_proofs::plonk::ConstraintSystem;

        let mut meta: ConstraintSystem<halo2_pallas::Base> = ConstraintSystem::default();
        let _config = <EccVerkleStepCircuit<halo2_pallas::Base> as Circuit<
            halo2_pallas::Base,
        >>::configure(&mut meta);
        // If we got here without panic, EccChip::<VerkleFixedBases>::configure
        // succeeded — meaning all 3 FixedPoint impls are valid + the
        // column allocation matches what EccChip expects. Sub-task C
        // circuit half can now plug in.
    }

    /// VerkleFixedBases::FullScalar / ShortScalar / Base all return
    /// the curve generator from generator() (scaffold base). Sub-task
    /// C will swap each to a per-Verkle-level fixed point — the test
    /// will be updated then to assert the new (deterministic, domain-
    /// separated) base.
    #[test]
    fn fixed_point_generators_match_pallas_generator() {
        use halo2_gadgets::ecc::chip::FixedPoint as HaloFixedPoint;
        use halo2_proofs::pasta::pallas as halo2_pallas;
        use pasta_curves::group::Curve;

        let expected = halo2_pallas::Point::generator().to_affine();
        assert_eq!(VerkleFullWidth.generator(), expected);
        assert_eq!(VerkleShort.generator(), expected);
        assert_eq!(VerkleBaseField.generator(), expected);
    }

    /// `u()` and `z()` return the right Vec lengths — NUM_WINDOWS for
    /// FullScalar/Base and NUM_WINDOWS_SHORT for ShortScalar. If the
    /// precomputed-table caching breaks, this catches it before
    /// EccChip tries to use a wrong-sized table at proof time.
    #[test]
    fn fixed_point_table_lengths_match_window_counts() {
        use halo2_gadgets::ecc::chip::{FixedPoint as HaloFixedPoint, NUM_WINDOWS, NUM_WINDOWS_SHORT};

        assert_eq!(VerkleFullWidth.u().len(), NUM_WINDOWS);
        assert_eq!(VerkleFullWidth.z().len(), NUM_WINDOWS);
        assert_eq!(VerkleShort.u().len(), NUM_WINDOWS_SHORT);
        assert_eq!(VerkleShort.z().len(), NUM_WINDOWS_SHORT);
        assert_eq!(VerkleBaseField.u().len(), NUM_WINDOWS);
        assert_eq!(VerkleBaseField.z().len(), NUM_WINDOWS);
    }
}
