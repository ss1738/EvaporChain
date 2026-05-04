//! Singh Counter-Decay Insurance (SCDI).
//!
//! ## What this is
//!
//! Insurance whose **premium grows linearly with policy age**
//! (loyalty cost: the longer you've held coverage, the more
//! you've paid in) AND whose **payout-cap grows linearly with
//! policy age** (loyalty reward: long-held policies have
//! larger covered payouts). Claim freshness still gates: a
//! stale claim with low proof-energy is rejected.
//!
//! ## How this differs from Energy-Clocked Coverage
//!
//! `evaporchain-energy-coverage` (Tier-3) has:
//!   - premium grows with age
//!   - payout SHRINKS with claim staleness
//!
//! SCDI has:
//!   - premium grows with age
//!   - payout-cap GROWS with policy age (counter-decay)
//!   - payout still gated by claim freshness via floor
//!
//! Same mechanism (energy-aware claim gate); opposite economic
//! shape on the payout side. SCDI rewards LOYALTY; ECC rewards
//! FRESHNESS-of-incident.
//!
//! ## Three structural decisions enforced as tests
//!
//! 1. **Premium is monotone in age.** Each tick adds; never
//!    subtracts. Property test confirms.
//!
//! 2. **Payout cap is monotone in age.** `cap(t) = base + slope · age`.
//!    Older policies have higher caps.
//!
//! 3. **Claim gate is still freshness-bound.** `claim_energy ≥
//!    claim_floor` required, regardless of cap growth. The
//!    counter-decay shape is on the *payout side*, not the
//!    *claim-acceptance side*.
//!
//! ## Module map
//!
//! - [`policy`] — [`Policy`] state machine.

pub mod policy;

pub use policy::{Policy, PolicyError, PolicyId, PolicyState};
