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
//! - [`witness`] — `StepWitness { state_hash, step_energy,
//!   epochs_elapsed }` — the per-step input the prover commits to.
//! - [`folded`] — `FoldedInstance { acc_hash, total_energy_remaining,
//!   step_count, latest_epoch }`.
//! - [`fold`] — `fold(prev, step, λ)` accumulates a step into the
//!   folded instance.
//! - [`verify`] — `verify_folded(instance, expected_acc_hash,
//!   expected_remaining_energy)` — substrate verifier (hash + energy
//!   bound; real Nova R1CS check plugs in later).
//!
//! The `acc_hash` field uses a domain-separated blake3 chain rather
//! than the real Nova folding arithmetic over a curve — substrate
//! quality, sufficient for downstream consumers to type-check
//! against the API. The cryptographic strengthening to actual
//! Nova/HyperNova folding is a separate commit (gated on the
//! arkworks integration in `evaporchain-proving`).

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
