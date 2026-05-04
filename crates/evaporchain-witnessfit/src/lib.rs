//! WitnessFit — wearable + chain attestation streaks.
//!
//! Per `research/INVENTION_STACK.md` §A5.5:
//!
//! > Wearable + chain attestation streaks. 100M+ wearable owners.
//!
//! > **Naming guardrail (§A5.7):** Consumer brand: WitnessFit.
//! > Primitive: Singh Streak. Staking variant: Decay-Bond.
//!
//! ## What's structurally different from Strava / Streaks / Apple Fitness
//!
//! Existing streak apps have a **binary failure mode**: miss a day,
//! the streak resets to zero. The cliff is harsh; the user-loss is
//! permanent; the on-app pressure is anxiety-shaped.
//!
//! Singh-Streak replaces the cliff with **graceful decay**. Three
//! structural decisions:
//!
//! 1. **Streak strength is a continuous quantity in [0, 1]**, not a
//!    day-count integer. Each successful check-in *adds* energy
//!    (capped at 1.0); every epoch of silence *decays* it on a
//!    chain-controlled half-life. A user who exercises 20 days then
//!    misses 2 doesn't lose everything — they're at maybe 0.85 — and
//!    a single check-in puts them back near full.
//!
//! 2. **Two thresholds, not one.** A streak is `Healthy` (≥ 0.75),
//!    `Aging` (0.40 ≤ x < 0.75), or `Broken` (< 0.40). The cliff
//!    only happens at Broken; the warning sits at Aging. The wallet
//!    UX nudges at Aging — *before* the user has lost everything.
//!
//! 3. **Witness data is opaque on chain.** WitnessFit doesn't store
//!    what the user actually did (run / lift / steps / heart-rate);
//!    only a Blake3 commitment over the device-attested payload +
//!    timestamp. The wearable does the assertion off-chain (HealthKit
//!    / Google Fit / Whoop); the chain only knows that *something
//!    happened* at epoch T and verifies the device's signature.
//!
//! ## Singh names per A5.7 guardrail
//!
//! - **WitnessFit** — consumer-facing brand name
//! - **Singh-Streak** — the primitive itself
//! - **Decay-Bond** — staking variant (V2; not in this crate)
//!
//! ## Module map
//!
//! - [`witness`] — [`WitnessAttestation`] device-signed payload commitment
//! - [`streak`] — [`Streak`] live state + lifecycle; `check_in`,
//!   `strength_at`, `bucket_at`
//! - [`bucket`] — [`StrengthBucket`] qualitative state ({Healthy,
//!   Aging, Broken}) with thresholds

pub mod bucket;
pub mod streak;
pub mod witness;

pub use bucket::{bucket_for_strength, StrengthBucket, AGING_THRESHOLD_BP, BROKEN_THRESHOLD_BP};
pub use streak::{Streak, StreakError, StreakId};
pub use witness::{WitnessAttestation, WitnessError};
