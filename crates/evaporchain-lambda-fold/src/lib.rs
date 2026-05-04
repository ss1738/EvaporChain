//! Lambda-Fold (Energy-Folded Light Client) — substrate.
//!
//! Per `research/INVENTION_STACK.md` §4.1 row 8:
//!
//! > **Lambda-Fold (Energy-Folded Light Client)** — First sublinear-
//! > in-active-energy verifier. Nova extension where each fold step
//! > folds the energy state. Decade-defining if the math holds.
//!
//! ## Why this is meaningful
//!
//! Nova (Kothapalli, Setty, Tzialla 2022) folds many R1CS instances
//! into a single one with O(log n) verifier work — i.e. a step-counter
//! never appears in the verifier's runtime. Lambda-Fold extends Nova
//! by carrying the chain's *energy accumulator* alongside the folded
//! R1CS instance. The verifier's final check includes:
//!
//!   1. The folded R1CS witness (existing Nova check).
//!   2. The cumulative λ-decay of energy across the folded steps.
//!
//! Together this makes a light client that verifies *both* state
//! correctness AND chain energy decay in O(log n) — the first
//! "sublinear-in-active-energy" verifier.
//!
//! ## What this crate ships
//!
//! Two parallel paths, chosen at chain startup via the
//! `lambda_fold_mode` governance flag (Phase 5.2 of
//! `LAMBDA_FOLD_NOVA_PLAN.md`):
//!
//! **Substrate path** (default, always available):
//! - [`witness`] — `StepWitness { state_hash, step_energy,
//!   epochs_elapsed }` — the per-step input the prover commits to.
//! - [`folded`] — `FoldedInstance { acc_hash, total_energy_remaining,
//!   step_count, latest_epoch }`.
//! - [`fold`] — `fold(prev, step, λ)` accumulates a step via a
//!   domain-separated blake3 hash chain. Cheap, deterministic, and
//!   serves as the fall-back authoritative accumulator.
//! - [`verify`] — `verify_folded(instance, expected_acc_hash,
//!   expected_remaining_energy)` — hash + energy bound check.
//!
//! **Nova path** (`feature = "nova"`, shipped 2026-05-04):
//! - [`nova_path::NovaFolder`] wraps `evaporchain-proving`'s
//!   `RealBlockProver` (Nova IVC at arity 8 with Poseidon-bound
//!   state root and a 5-equation chain-aggregate energy-fold
//!   gadget). One folder per chain — the heavy `pp` setup runs
//!   once.
//! - [`nova_path::NovaFoldedInstance`] carries a
//!   bincode-serialized `CompressedSNARK` proof (replaces blake3
//!   `acc_hash`) plus the same `total_energy_remaining`,
//!   `step_count`, `latest_epoch` fields as the substrate
//!   `FoldedInstance` for hot-path reads without verification.
//! - [`nova_path::verify_nova_folded`] runs the light-client
//!   verifier against preprocessed `vk_bytes` only — no `pp`,
//!   no prover state — and decides validity in **~23 ms at 100
//!   folds** on M4 (verify(100)/verify(10) = 1.083, essentially
//!   flat per `LAMBDA_FOLD_NOVA_PLAN.md` Phase 6.1).
//!
//! The Nova path is the production cryptographic strengthening of
//! the substrate path. The substrate path is retained because it's
//! fast to build (no nova-snark / halo2curves transitive deps when
//! the `nova` feature is off) and because Phase 5.3's call-site
//! contract runs the substrate fold ALWAYS — the Nova path is
//! additive, not replacing.

pub mod fold;
pub mod folded;
pub mod verify;
pub mod witness;

// Phase 4 of LAMBDA_FOLD_NOVA_PLAN — Nova-backed fold/verify path
// behind the `nova` feature. The substrate blake3 path above stays
// available; Phase 5's `lambda_fold_mode` governance flag chooses
// which one runs at chain startup.
#[cfg(feature = "nova")]
pub mod nova_path;

pub use fold::{fold, FoldError};
pub use folded::FoldedInstance;
pub use verify::{verify_folded, VerifyError};
pub use witness::StepWitness;

#[cfg(feature = "nova")]
pub use nova_path::{
    verify_nova_folded, NovaFoldError, NovaFoldedInstance, NovaFolder, NovaVerifyError,
};

// Phase 5.1 — re-export the witness type consumers need to feed
// `NovaFolder::fold_block` so they don't have to depend on
// `evaporchain-proving` directly.
#[cfg(feature = "nova")]
pub use evaporchain_proving::nova::ThermodynamicWitness as NovaThermodynamicWitness;
