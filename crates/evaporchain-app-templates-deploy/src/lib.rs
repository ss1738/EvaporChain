//! Deploy-request layer for the app-templates catalogue.
//!
//! `evaporchain-app-templates` registers 20 application primitives
//! and lets the dApp layer enumerate them. This crate is the *next
//! step*: it produces a `DeployRequest` — a structured payload the
//! chain's contract layer (ContractEngine, EvaporScript) can
//! materialise into an actual on-chain instance.
//!
//! ## What this crate does
//!
//! 1. **Builds** a `DeployRequest { template_class, params, deployer,
//!    submitted_at_epoch, nonce }`.
//! 2. **Validates** the request against the template's catalogue
//!    descriptor — every required parameter key must be present;
//!    extra keys are warned but not rejected (forward-compat).
//! 3. **Emits canonical bytes** for the request — a deterministic
//!    serialization validators can hash + sign + agree on. Includes
//!    a domain-separation tag and the template class id, so a deploy
//!    signature can never be replayed for any other purpose.
//! 4. **Computes the BLAKE3 commitment** consumers can use as the
//!    deploy-tx hash.
//!
//! ## What this crate does NOT do
//!
//! - It does NOT execute the contract. Materialisation happens in
//!   the existing `ContractEngine` path; this crate just produces the
//!   payload that path consumes.
//! - It does NOT verify signatures. The signature scheme is whatever
//!   the chain runs (BLS / ML-DSA / Ed25519); the higher transaction
//!   layer plugs verification in.
//! - It does NOT touch state. Pure-function serialization +
//!   validation only.
//!
//! ## Module map
//!
//! - [`request`] — [`DeployRequest`] struct + canonical bytes +
//!   commitment.
//! - [`validate`] — [`validate_against_descriptor`] schema check.
//! - [`required_keys`] — [`required_keys_for`]: the minimal key set
//!   each template's `params` must contain. Lives here (not in
//!   app-templates) because it's the *deploy-time contract*, not the
//!   *catalogue-display contract*.

pub mod request;
pub mod required_keys;
pub mod validate;

pub use request::{DeployRequest, RequestError};
pub use required_keys::required_keys_for;
pub use validate::{validate_against_descriptor, ValidationError};

#[cfg(test)]
mod press_claim_tests {
    use super::*;
    use evaporchain_app_templates::class::MAYFLY;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "DeployRequest is the typed payload between dApp
    /// and chain. Construction rejects out-of-range template classes
    /// and non-object params. `commitment()` is BLAKE3 over canonical
    /// (sorted-key) signing bytes — same request → same commitment
    /// across validators, runtimes, and indexers."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let params = serde_json::json!({"initial_energy": 1000, "half_life": 100});
        let r = DeployRequest::new(MAYFLY, params.clone(), [7u8; 32], 100, 0).unwrap();
        let c1 = r.commitment().unwrap();
        let c2 = r.commitment().unwrap();
        // Validator-determinism: same input → same commitment.
        assert_eq!(c1, c2);

        // Out-of-range template class is rejected.
        let bad = evaporchain_app_templates::TemplateClass(0xFFFF_FFFF);
        assert!(DeployRequest::new(bad, params.clone(), [7u8; 32], 100, 0).is_err());

        // Non-object params are rejected.
        let nonobj = serde_json::json!(42);
        assert!(DeployRequest::new(MAYFLY, nonobj, [7u8; 32], 100, 0).is_err());
    }
}
