//! Materialiser — turns a validated [`DeployRequest`] into a
//! [`MaterialiseInstruction`] the chain's `ContractEngine` dispatches.
//!
//! `app-templates-deploy` produces a signable, validatable deploy
//! payload. This crate is the *next step*: it derives a
//! deterministic on-chain instance id and packages the typed init
//! calldata so the contract engine can construct the actual
//! application instance.
//!
//! ## What this crate does
//!
//! 1. **Derives a deterministic instance id** from
//!    `(template_class, deployer, nonce)` so two validators
//!    independently produce the same on-chain handle.
//! 2. **Re-runs schema validation** at materialise-time. The deploy
//!    layer validates at submission; the materialiser validates
//!    again at execution. Two-phase validation prevents a request
//!    that *looked* schema-valid at submit but referenced a
//!    descriptor that has since been hot-swapped (forward-compat
//!    safety).
//! 3. **Emits canonical init calldata** — the params re-serialised
//!    with the same canonicalization rules the deploy commitment
//!    used. The contract engine consumes this and dispatches to
//!    the typed materialiser for the specific template class.
//!
//! ## What this crate does NOT do
//!
//! - It does NOT execute the contract. That's the contract engine's
//!   job; this crate produces the dispatch envelope.
//! - It does NOT verify the deploy signature. That's the transaction
//!   layer.
//! - It does NOT touch state. Pure-function only — the same
//!   `DeployRequest` always produces the same `MaterialiseInstruction`.
//!
//! ## Module map
//!
//! - [`instance`] — [`InstanceId`] and the deterministic derivation.
//! - [`materialise`] — [`MaterialiseInstruction`] +
//!   [`materialise_request`] driver function.

pub mod instance;
pub mod materialise;

pub use instance::{derive_instance_id, InstanceId, INSTANCE_DOMAIN_TAG};
pub use materialise::{materialise_request, MaterialiseError, MaterialiseInstruction};

#[cfg(test)]
mod press_claim_tests {
    use super::*;
    use evaporchain_app_templates::class::{MAYFLY, SINGH_SABI};

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "InstanceId derivation is deterministic — same
    /// (template_class, deployer, nonce) → same id, byte-exact, on
    /// every validator. Different inputs → different ids: changing
    /// the template class, deployer, or nonce all produce distinct
    /// ids."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let deployer = [7u8; 32];
        let id1 = derive_instance_id(MAYFLY, &deployer, 0);
        let id1_again = derive_instance_id(MAYFLY, &deployer, 0);
        assert_eq!(id1, id1_again, "instance id must be deterministic");

        // Different class → different id.
        let id2 = derive_instance_id(SINGH_SABI, &deployer, 0);
        assert_ne!(id1, id2);

        // Different deployer → different id.
        let id3 = derive_instance_id(MAYFLY, &[8u8; 32], 0);
        assert_ne!(id1, id3);

        // Different nonce → different id.
        let id4 = derive_instance_id(MAYFLY, &deployer, 1);
        assert_ne!(id1, id4);
    }
}
