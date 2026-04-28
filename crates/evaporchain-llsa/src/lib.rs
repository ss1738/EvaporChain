//! Lambda-Locked Self-Amendment (LLSA).
//!
//! Per `research/INVENTION_STACK.md` §A1.2 T4 (Tier-0 theorem-grade):
//!
//! > **Lambda-Locked Self-Amendment (LLSA)** — protocol upgrades
//! > require a Coq/Lean term of type
//! > `forall s, Inv(s) → Inv(step_new(s))`.
//! > Pinned MetaCoq kernel + extraction-to-Rust.
//! >
//! > "Our upgrades are gated by mechanically-checked invariant-
//! > preservation proofs — Tezos has self-amendment without proofs;
//! > we are the first chain whose governance is a theorem."
//!
//! ## Trust boundary (per doctrine §10.13)
//!
//! > "Löb's theorem is real. Don't claim the chain has escaped Gödel.
//! > The Coq kernel is an external TCB; document it honestly."
//!
//! Concretely: this crate does **not** itself run Coq. The on-chain
//! gate verifies (1) the proof artefact's *binding* (hash of the proof
//! bytes matches the on-chain commitment) and (2) the artefact's
//! *target invariant* matches the chain's expected invariant id.
//! The actual Coq term checking happens off-chain in a pinned
//! MetaCoq kernel; the resulting proof bytes (the kernel's
//! certificate that the term type-checks) are what get supplied here.
//!
//! Exchanging the verifier (e.g. Coq → Lean) is just swapping the
//! trait implementation supplied to [`apply_amendment`].
//!
//! ## Module map
//!
//! - [`proof`] — `LlsaProof` artefact + `ProofVerifier` trait.
//! - [`amendment`] — `Amendment { from_version, to_version, proof,
//!   step_new_descriptor }`.
//! - [`apply`] — `apply_amendment(registry, amendment, verifier)`
//!   gates the EPV registry's transition.

pub mod amendment;
pub mod apply;
pub mod proof;

pub use amendment::{Amendment, AmendmentError};
pub use apply::apply_amendment;
pub use proof::{LlsaProof, ProofError, ProofVerifier};
