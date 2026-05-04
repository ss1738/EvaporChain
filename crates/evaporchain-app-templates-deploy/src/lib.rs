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
