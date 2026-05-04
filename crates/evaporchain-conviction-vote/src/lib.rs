//! Evaporating Conviction Vote — governance with decay-bounded patience.
//!
//! ## Standard conviction voting (Commons Stack 2018)
//!
//! Voters allocate `stake` to a proposal. Conviction integrates
//! over time toward an asymptote of `stake / (1 - α)` (where α
//! is the conviction-growth parameter, typically α = 0.9 per
//! tick). A proposal passes when its accumulated conviction
//! exceeds a `threshold`. Voters can re-allocate at any time;
//! conviction reverts toward zero on withdrawal.
//!
//! Implication: standard conviction voting requires a voter to
//! "sit on" a proposal patiently for the conviction to build —
//! a Schelling point against governance flash-mobs.
//!
//! ## What "evaporating" adds
//!
//! In standard conviction, the voter's `stake` is fixed once
//! deposited. In the Evaporating variant, **the stake itself
//! decays** under the chain's single-λ. The conviction integrand
//! becomes:
//!
//!   c(t+1) = α · c(t) + (1 - α) · stake(t)
//!
//! where `stake(t)` is the energy-decayed stake at tick `t`.
//!
//! Implication: conviction has TWO time scales —
//! - Conviction *integration*: grows with time (faster with high α).
//! - Stake *decay*: shrinks with time (faster with high λ).
//!
//! When `λ > (1 - α)`, the decay outpaces the integration: a
//! voter who deposits-and-leaves sees their conviction peak and
//! then *decline*. A proposal that requires sustained conviction
//! requires sustained engagement (re-staking) to keep its
//! supporters' weight fresh.
//!
//! ## Three structural decisions enforced as tests
//!
//! 1. **Pure-integer fixed-point.** Conviction in micros (1.0 =
//!    10⁶ micros); α as a fixed-point ratio out of 1_000_000.
//!    Validator-deterministic byte-equality.
//!
//! 2. **Bounded asymptote with decay-aware ceiling.** The
//!    conviction at time t is bounded above by `stake(t) /
//!    (1 - α)`. With `stake(t)` decaying, the asymptote shrinks.
//!
//! 3. **Pass-then-stay-passed only via threshold-at-tick.**
//!    Conviction passing the threshold mints a one-shot pass
//!    event; subsequent decay below threshold doesn't un-pass
//!    (the higher layer's effect was applied). This prevents
//!    pass/un-pass flapping.
//!
//! ## What this crate does NOT do
//!
//! - It does NOT model the actual proposal payload. The chain
//!   wraps proposal data; this crate tracks the conviction state.
//! - It does NOT perform the chain's decay tick. Caller passes
//!   the energy-adjusted stake at each `tick` call.
//! - It does NOT enforce stake-uniqueness across proposals. A
//!   voter is free to allocate to multiple proposals; the chain's
//!   higher layer manages "total stake ≤ voter balance".
//!
//! ## Module map
//!
//! - [`proposal`] — [`Proposal`] state machine.
//! - [`registry`] — [`ConvictionRegistry`] for per-proposal +
//!   per-voter tracking.

pub mod proposal;
pub mod registry;

pub use proposal::{ALPHA_MICROS_DEFAULT, Proposal, ProposalError, ProposalId, MICROS};
pub use registry::{ConvictionRegistry, RegistryError, VoterId};
