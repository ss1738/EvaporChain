//! Per-primitive invariant-validation layer.
//!
//! Sits between [`evaporchain-app-templates-engine`]'s typed
//! `TypedInit` (JSON parsed into typed fields) and the chain's
//! `ContractEngine` (which calls per-primitive constructors). For
//! each variant of `TypedInit`, runs the per-primitive invariant
//! checks the constructor would normally enforce — `initial > 0`,
//! `floor < initial`, `m ≤ n`, `ladder sorted by days`, etc. — so
//! that bad deploys fail at this pre-flight gate, *before* any
//! chain state is touched.
//!
//! Returns [`Bound`] — a `TypedInit` that's been pre-validated
//! for all known invariant failures.
//!
//! ## Why a separate crate (not part of `engine`)
//!
//! `engine` answers "is this JSON parseable into the right types?"
//! `bind` answers "are the values in those types acceptable to the
//! primitive's constructor?" Splitting keeps each crate one job:
//! type-shaped vs value-shaped validation.
//!
//! ## What this crate does NOT do
//!
//! - It does NOT call the per-primitive constructors. Those
//!   constructors live in the primitive's own crate and require
//!   chain runtime context (current_epoch, owner, ids) we don't
//!   have here. The chain's `ContractEngine` does that step.
//! - It does NOT depend on the 20 primitive crates. Invariant
//!   *values* are duplicated here as numeric constants — the
//!   tradeoff is "two places to update if a primitive's bound
//!   changes" vs "this crate compiles in seconds, not minutes."
//!   The cross-check is the primitive's own constructor, which
//!   re-validates; if the bound moves, the primitive's tests catch
//!   it and bind's tests fail in CI.
//!
//! ## Module map
//!
//! - [`context`] — [`BindContext`]: deployer + instance_id +
//!   current_epoch the chain provides at materialise time.
//! - [`bind`] — [`Bound`] enum + [`bind`] driver function +
//!   [`BindError`].

pub mod bind;
pub mod context;

pub use bind::{bind, Bound, BindError};
pub use context::BindContext;

#[cfg(test)]
mod press_claim_tests {
    use super::*;
    use evaporchain_app_templates_engine::{init_mayfly, TypedInit};

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "Bind is the per-primitive value-shaped
    /// invariant gate between the engine's type-shaped TypedInit
    /// and the chain's per-primitive constructors. A valid Mayfly
    /// init binds; an invalid one (zero half_life, etc.) is
    /// rejected with a typed BindError before any state is touched."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Valid Mayfly init: positive initial_energy + half_life.
        let valid = TypedInit::Mayfly(init_mayfly::InitConfig {
            initial_energy: 1_000,
            half_life: 100,
        });
        assert!(bind(valid).is_ok());

        // Invalid: zero half_life violates the per-primitive bound.
        let invalid = TypedInit::Mayfly(init_mayfly::InitConfig {
            initial_energy: 1_000,
            half_life: 0,
        });
        assert!(bind(invalid).is_err());
    }
}
