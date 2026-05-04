//! IB Validators V2 — structural jail layer over the V1 vote gate.
//!
//! V1 (`evaporchain-ib-validators`) ships `ib_vote(local, prior,
//! params) → Commit | Abstain` based on KL divergence. It says
//! nothing about *which* validators are eligible to vote — any
//! validator that runs the gate gets a verdict.
//!
//! V2 wraps the V1 gate with three structural rejection paths:
//!
//! 1. **CHSH-failed-window jail** — a validator that was active
//!    during an epoch where the chain's `BellCertificate` failed
//!    is jailed for `jail_epochs` epochs. Bell-Beacon V2 supplies
//!    the failure signal; this crate consumes the per-validator
//!    activity trace and toggles jail state.
//!
//! 2. **Energy-floor jail** — a validator whose energy has decayed
//!    below `energy_floor` cannot vote until refreshed. Pulls in
//!    EvaporChain's energy primitive at the validator-set layer.
//!
//! 3. **Explicit slash** — operator can jail a validator with a
//!    typed reason (double-sign, equivocation, manual ban). Same
//!    deterministic expiry rules.
//!
//! ## Validator-determinism
//!
//! - `JailState` stores entries in a `BTreeMap<ValidatorId, _>`
//!   so iteration order is canonical.
//! - Jail expiry is computed against a single `current_epoch` field;
//!   no wall-clock dependency.
//! - `ib_vote_v2` is a pure function of `(local, prior, params,
//!   validator_id, energy, jail_state, current_epoch)`.

pub mod jail;
pub mod vote;

pub use jail::{JailEntry, JailReason, JailState, ValidatorId};
pub use vote::{ib_vote_v2, VoteV2, VoteV2Error};
