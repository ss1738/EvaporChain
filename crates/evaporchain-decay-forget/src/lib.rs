//! Decay-Forget Proofs (Tier 2).
//!
//! Per `research/INVENTION_STACK.md` §4.2 (V2 primitives):
//!
//! > **Decay-Forget Proofs** — GDPR-native — chain *provably cannot*
//! > recover a timestamp once decayed past threshold.
//!
//! ## Design
//!
//! Negative dual of [`evaporchain_prp`]:
//!
//! - **PRP** (positive): proves a record IS retained at `query_epoch`
//!   under `committed_energy` and the chain-global λ.
//! - **Decay-Forget** (negative): proves a record's *recoverability
//!   commitment* has decayed below the chain-set forget threshold and
//!   is therefore *cryptographically un-recoverable* at `query_epoch`.
//!
//! GDPR's "right to be forgotten" demands that data subjects' records
//! be physically un-recoverable on demand. EvaporChain implements this
//! as a default-on property: a record's recoverability commitment
//! decays with λ unless explicitly refreshed (which the data subject
//! controls by keeping the record alive in the refresh market). When
//! the commitment falls below `forget_threshold`, the chain produces
//! a `DecayForgetProof` that any auditor (including the regulator)
//! can verify in O(1).
//!
//! ## Module map
//!
//! - [`proof`] — `DecayForgetProof` artefact + errors.
//! - [`prove`] — `prove_forgotten(record_id, original_commitment, λ,
//!   activated_epoch, query_epoch, forget_threshold)`.
//! - [`verify`] — `verify_forget_proof(proof)` re-derives the witness
//!   and confirms the recoverability commitment is below threshold.

pub mod proof;
pub mod prove;
pub mod verify;

pub use proof::{DecayForgetProof, ForgetProofError};
pub use prove::prove_forgotten;
pub use verify::verify_forget_proof;
