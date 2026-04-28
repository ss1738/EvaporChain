//! Singh-Boltzmann Stake — validator stake decays by default; refresh
//! by producing blocks.
//!
//! Per `research/INVENTION_STACK.md` §4.1 row 5:
//!
//! > **Singh-Boltzmann Stake** — Validator stake decays by default;
//! > refresh by producing blocks. **Kills the stake-and-lease-key-to-
//! > MEV pattern.**
//!
//! And per the mechanism-design source attribution: "Boltzmann
//! distribution over validator activity."
//!
//! ## Mechanics
//!
//! 1. **Decay** — every epoch, each validator's *active stake* decays
//!    via the chain-global `energy_at_epoch(stake, λ, 1)`. A validator
//!    that does nothing watches its voting power approach zero on the
//!    same time-constant as every other state object.
//! 2. **Refresh** — producing a block credits the validator's stake.
//!    The credit is sized so a validator producing at the *expected
//!    rate* exactly compensates the decay (steady state).
//! 3. **Boltzmann ranking** — selection probability for the next
//!    proposer is `∝ exp(activity_score) · stake`, where activity_score
//!    is recent block-production rate. This is the same Boltzmann
//!    machinery as `evaporchain-cfm::weight::boltzmann_weight` — high
//!    activity = high effective stake.
//!
//! ## Why this kills the stake-and-lease-key-to-MEV pattern
//!
//! In existing PoS chains a holder can stake once, lease the signing
//! key to a third-party MEV operator, and collect base yield without
//! producing blocks themselves. The lease has no liveness obligation.
//!
//! Under Singh-Boltzmann Stake the *holder's* stake decays unless they
//! sign blocks. A passive holder watches their voting power evaporate;
//! a leaseholder MEV operator can sustain their own stake (because
//! they do produce blocks) but cannot resurrect the original holder's.
//! There is no "set it and forget it" stake.
//!
//! ## Module map
//!
//! - [`stake_state`] — `ValidatorStake { active, last_touched_epoch }`.
//! - [`decay`] — `decay_validator_stake(stake, λ, current_epoch)`
//!   advances a validator's active stake to the current epoch.
//! - [`refresh`] — `refresh_on_block(stake, refresh_amount, epoch)`
//!   credits stake when the validator produces a block.
//! - [`boltzmann`] — `proposer_weight(stake, activity_score, beta_mb)`
//!   computes the Boltzmann-weighted selection probability.

pub mod boltzmann;
pub mod decay;
pub mod refresh;
pub mod stake_state;

pub use boltzmann::proposer_weight;
pub use decay::decay_validator_stake;
pub use refresh::refresh_on_block;
pub use stake_state::ValidatorStake;
