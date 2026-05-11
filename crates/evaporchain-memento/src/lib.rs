//! Memento contracts — commit-and-forget cryptographic substrate.
//!
//! Per `IMPOSSIBLE_RESEARCH_STACK.md` Branch 4 — Decay-Native
//! Architecture, "Memento contracts":
//!
//! > "Contracts that hold sealed state, reveal only when conditions
//! > trigger (e.g., wallet inactive 5 years → execute will).
//! > EvaporScript primitive `Memento { trigger, reveal }`."
//!
//! # What a Memento is
//!
//! A `MementoContract` is two pieces of data:
//!
//! 1. **A commitment** — `BLAKE3("evaporchain-memento-v1" || payload
//!    || nonce)`. The payload itself never lives on-chain; only the
//!    32-byte commitment does. This is information-theoretic
//!    hiding (with computational binding under BLAKE3 collision
//!    resistance).
//!
//! 2. **A trigger** — a [`MementoTrigger`] enum describing the
//!    condition that, once witnessed by the chain, permits a
//!    [`MementoReveal`] witness to decommit the payload.
//!
//! # Why it's novel
//!
//! Bitcoin has timelocks (`OP_CHECKLOCKTIMEVERIFY`); Ethereum has
//! commit-reveal schemes. Neither chain has a contract primitive
//! whose unlock condition is *thermodynamic state* — wallet idleness,
//! energy decay below a floor. EvaporChain's energy-decay substrate
//! makes those conditions cheap to verify on-chain because they're
//! native to the state model.
//!
//! Concretely, [`MementoTrigger`] supports:
//!
//! - [`MementoTrigger::BlockHeightReached`] — classical timelock.
//! - [`MementoTrigger::OwnerInactiveSince`] — wallet has not been the
//!   sender of any transaction for `min_idle_epochs` epochs since
//!   `since_epoch`. Maps to the "wallet inactive 5 years → execute
//!   will" use case.
//! - [`MementoTrigger::OwnerEnergyBelow`] — the owner's account
//!   energy has decayed below a threshold. The thermodynamically
//!   native trigger; only EvaporChain can offer this primitive.
//! - [`MementoTrigger::OwnerSignedReveal`] — the owner has explicitly
//!   produced a reveal-authorising signature. Lets the owner short-
//!   circuit the wait.
//! - [`MementoTrigger::AttesterApproval`] — a pre-registered attester
//!   address has approved the reveal. Multi-signature-style escape
//!   hatch.
//!
//! # Threat model
//!
//! - **Hiding** — adversary observing the chain cannot recover the
//!   payload from the commitment alone, even with unbounded compute,
//!   *as long as the nonce has ≥256 bits of entropy*. This is the
//!   information-theoretic guarantee of a domain-separated BLAKE3
//!   commitment with a uniform nonce.
//! - **Binding** — the contract creator cannot produce a second
//!   `(payload', nonce')` decommitment to the same commitment without
//!   finding a BLAKE3 collision (2^128 work).
//! - **Liveness** — the contract's reveal trigger must be observable
//!   by the chain. A bug in the trigger predicate (e.g. wrong epoch
//!   accounting) would make a Memento permanently unrevealable; the
//!   chain treats this as the "evaporated" terminal state.
//!
//! # What this crate ships
//!
//! - [`MementoContract`] — the sealed-on-chain contract object
//! - [`MementoTrigger`] — the 5 trigger variants
//! - [`MementoReveal`] — the off-chain witness the reveal claimant
//!   submits
//! - [`seal`] / [`try_reveal`] — the two top-level operations
//! - [`ChainObservation`] — the chain-state view the trigger
//!   predicates consume (a `&dyn` would couple this crate to the
//!   chain's StateDB; the read-only struct approach keeps the
//!   primitive standalone-testable)
//! - [`RevealError`] — typed errors covering commitment mismatch,
//!   trigger unsatisfied, replay, and structural failure

#![deny(unsafe_code)]
#![warn(missing_docs)]

mod commitment;
mod reveal;
mod trigger;

pub use commitment::{seal, MementoCommitment, MementoContract, MementoVersion};
pub use reveal::{try_reveal, ChainObservation, MementoReveal, RevealError};
pub use trigger::{MementoTrigger, TriggerError};
