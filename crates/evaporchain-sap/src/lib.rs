//! Singh Attention Pool (SAP).
//!
//! Per `research/INVENTION_STACK.md` §A5.2:
//!
//! > Each verified human wallet emits Attention Quanta (AQ) per
//! > minute, capped, decaying at λ_attn ≈ 45min (empirical forgetting
//! > curve). Advertisers/creators bid energy into a pool that pays
//! > out only against AQs whose decay-state is above threshold at
//! > attestation moment. **A 5-minute-old AQ is worth more than a
//! > 25-min one** because the human is statistically still
//! > cognitively-engaged.
//!
//! > **Pitch:** *"For the first time, your attention has a half-life
//! > — and a price."* NYT Business desk material.
//!
//! ## What V1 ships (and what it doesn't)
//!
//! Per A5.2 doctrine, the hardest part is the gaze-attestation TEE
//! circuit. **V1 defers the TEE.** This crate ships the
//! cryptoeconomic primitive — minting, decay, pool, payout — leaving
//! the gaze attestation behind a verifier closure (the same
//! generic-over-signature pattern Singh-Posthuma + WitnessFit use).
//! When the TEE lands as V2, it plugs in without changing the
//! lifecycle math.
//!
//! Three structural decisions:
//!
//! 1. **AQs decay on a forgetting-curve half-life.** Default 45
//!    minutes per Ebbinghaus / FSRS; the chain stores `λ_attn` as
//!    the AQ's `half_life` and decays per the standard
//!    `energy_at_epoch` rule. A fresh AQ is worth its full minted
//!    value; a stale one is worth a fraction of that.
//!
//! 2. **Per-wallet rate cap.** A wallet cannot mint more than
//!    `max_aq_per_window` quanta in a `window_epochs` rolling
//!    window. This is the anti-Sybil guard: even with a verified-
//!    human attestation, a single human's attention is bounded.
//!    Validators agree on the rate-limited mint outcome.
//!
//! 3. **Payouts gate on a `min_freshness_bp` floor.** An advertiser
//!    bids `(max_cpm, freshness_floor)`. The pool clears via SDDC
//!    on the price axis; the SAP layer additionally filters AQs
//!    whose decayed value at the attestation moment is below
//!    `freshness_floor`. Gaze-stale AQs simply don't earn.
//!
//! ## Module map
//!
//! - [`quantum`] — [`AttentionQuantum`] mint + decayed-value query;
//!   [`AqMintError`].
//! - [`pool`] — [`AttentionPool`] per-wallet rate cap + audit trail.
//! - [`payout`] — [`payout_for`]: filters AQs by freshness floor and
//!   reports the total earnable energy.

pub mod payout;
pub mod pool;
pub mod quantum;

pub use payout::{payout_for, PayoutError, PayoutSummary};
pub use pool::{AttentionPool, PoolError};
pub use quantum::{AqId, AqMintError, AttentionQuantum};
