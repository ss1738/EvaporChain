//! Deploy-fee oracle for app-templates.
//!
//! Pure function: `TypedInit → u64` gas cost. Same input across
//! validators always produces the same fee — validator-deterministic
//! by construction (integer-only math, no float, no hashing of
//! mutable state, no time).
//!
//! ## Why an oracle, not a fixed table
//!
//! Some primitives have variable-cost params:
//! - **Singh-Lineage**: cost scales with ladder length (more rungs
//!   ⇒ more state ⇒ more gas)
//! - **SCL**: cost scales with verb + object string length (more
//!   bytes to store)
//! - **SGB / SSM**: cost scales with fragment string length
//!
//! A fixed per-class table would either over-charge simple deploys
//! or under-charge expensive ones. The oracle quotes the actual
//! deploy's complexity.
//!
//! ## Three structural decisions enforced as invariants
//!
//! 1. **Integer-only.** No float; all costs are `u64` derived from
//!    integer-base + integer-per-unit-complexity. Validators
//!    agree byte-for-byte.
//! 2. **Monotone in complexity.** Adding a ladder rung never
//!    decreases the Lineage fee; making a fragment longer never
//!    decreases the SGB fee. Tested via property tests.
//! 3. **Bounded.** All costs fit in `u64`. The invariant check at
//!    the bind layer caps inputs (ladder length, fragment length)
//!    so a malicious deploy can't trigger an overflow here. The
//!    oracle uses saturating arithmetic as a defense-in-depth
//!    second line.
//!
//! ## What this crate does NOT do
//!
//! - It does NOT charge the fee — that's the chain's transaction
//!   layer (debits the deployer's balance).
//! - It does NOT model dynamic fee markets — fees are deterministic
//!   per (TypedInit, current network base-fee multiplier). This
//!   crate returns the *complexity-proportional base cost*; a
//!   network multiplier is applied at the higher transaction layer.
//! - It does NOT model runtime / per-op gas. Only the one-time
//!   *deploy* cost. Per-op gas lives with the contract VM.
//!
//! ## Module map
//!
//! - [`oracle`] — [`fee_for`] driver + [`base_fee`] +
//!   per-primitive cost models.

pub mod oracle;

pub use oracle::{base_fee, fee_for, BASE_DEPLOY_FEE};
