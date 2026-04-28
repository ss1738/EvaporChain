//! Provable Retention Proofs (PRP).
//!
//! Per `research/INVENTION_STACK.md` §4.1 row 11:
//!
//! > **Provable Retention Proofs** — Positive-finality dual of #10
//! > [Evaporated-Fork Certificates]. Provable retention as a first-
//! > class operation. Regulator-survival primitive.
//!
//! ## Why this exists
//!
//! §3.2 of the doctrine names regulators as the chain's real
//! adversary. Regulators (MiCA, FATF, GDPR) sometimes mandate
//! *selective* permanent retention (e.g. of certain transaction
//! categories) — at odds with the chain's anti-feature manifesto
//! "no permanent data storage at the protocol level".
//!
//! PRP threads the needle: state is retained iff *enough energy was
//! committed to keep it alive*. A regulator who wants permanence
//! pays the energy cost; the chain produces a *proof* that the state
//! was alive at any queried epoch under the global λ.
//!
//! ## Substrate
//!
//! - [`proof`] — `RetentionProof { state_id, retained_until_epoch,
//!   committed_energy, witness }`.
//! - [`prove`] — `prove_retention(state_id, committed_energy, λ,
//!   activated_epoch)` computes the latest epoch at which the state
//!   was still above the chain-set retention floor.
//! - [`verify`] — `verify_retention_proof(proof, λ, query_epoch,
//!   floor)` returns true iff `query_epoch ≤ retained_until_epoch`
//!   AND the proof's witness re-derives correctly.

pub mod proof;
pub mod prove;
pub mod verify;

pub use proof::{RetentionProof, RetentionProofError};
pub use prove::prove_retention;
pub use verify::verify_retention_proof;
