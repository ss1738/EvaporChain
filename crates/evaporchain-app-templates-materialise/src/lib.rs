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
