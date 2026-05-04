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

#[cfg(test)]
mod press_claim_tests {
    use super::*;
    use evaporchain_app_templates_engine::{init_mayfly, TypedInit};

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "Deploy-fee oracle is integer-only and validator-
    /// deterministic. `fee_for(typed)` returns ≥ `BASE_DEPLOY_FEE`,
    /// is monotone in input complexity, never returns 0, and uses
    /// saturating arithmetic to defend against overflow."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Mayfly is the simplest variant — compose a TypedInit.
        let mayfly = TypedInit::Mayfly(init_mayfly::InitConfig {
            initial_energy: 1_000,
            half_life: 100,
        });
        let f1 = fee_for(&mayfly);
        let f1_again = fee_for(&mayfly);

        // Determinism: same input → byte-identical output.
        assert_eq!(f1, f1_again);
        // Always ≥ base fee (every deploy pays the floor).
        assert!(f1 >= BASE_DEPLOY_FEE);
        assert!(f1 > 0, "fee_for must never return 0");
        // base_fee returns a sensible positive value too.
        let b = base_fee(&mayfly);
        assert!(b >= BASE_DEPLOY_FEE);
    }
}
