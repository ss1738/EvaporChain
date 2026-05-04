//! Singh Decay-Dutch Continuous Auction (SDDC).
//!
//! Per `research/INVENTION_STACK.md` §A5.2:
//!
//! > Bidders commit not just price but their *willingness to hold*;
//! > high-λ-tolerant bidders win at lower prices. **Foundational
//! > mechanism**, not standalone market — underlies SAP, SCL, SFSV,
//! > SHLM. Build once, reuse everywhere.
//!
//! ## What's new vs a vanilla Dutch auction
//!
//! A classical Dutch auction descends price from a ceiling and clears
//! at the first bid that meets it. SDDC clears on **two axes**: price
//! AND λ-tolerance. The lot has a published decay rate `λ_lot`. A bid
//! must satisfy *both*:
//!
//! - `current_price(epoch) ≤ bid.max_price` — the price axis (Dutch)
//! - `λ_lot ≤ bid.lambda_tolerance` — the willingness-to-hold axis
//!
//! Bidders who tolerate higher decay rates self-select into the pool
//! earlier and at lower price floors, because they're willing to take
//! the riskier asset. The auction operator never has to compute who
//! "deserves" the discount — the joint clearing condition produces it
//! mechanically.
//!
//! ## Why "foundational"
//!
//! Every λ-aware marketplace in the doctrine clears through SDDC:
//!
//! - **SAP** (Singh Attention Pool) — advertisers bid `(max_cpm, max_AQ_age)`.
//! - **SCL** (Singh Capability Lease) — leasers bid `(max_rent, max_lease_λ)`.
//! - **SFSV** (Singh Future-Self Vault) — third parties bid for future-self
//!   claims with `(discount_price, max_decay_until_payout)`.
//! - **SHLM** (Singh Skill Half-Life Market) — employers bid for fresh-skill
//!   tokens with `(salary_offer, min_skill_λ_tolerance)`.
//!
//! All four collapse to "two-axis Dutch with the second axis bound to λ."
//!
//! ## Module map
//!
//! - [`auction`] — [`Auction`], [`AuctionStatus`], [`AuctionId`]; lifecycle.
//! - [`bid`] — [`Bid`] payload submitted by bidders.
//! - [`pricing`] — [`current_price`]: monotone-descending step curve.
//! - [`clearing`] — [`try_clear`]: first-valid-bid-by-submission-time wins.
//!
//! ## Determinism
//!
//! All pricing/clearing math is integer-only (`u64` / `u128` intermediates,
//! saturating where the squaring chain would otherwise overflow). No
//! floats; no system clock — every "now" is an explicit `Epoch` parameter
//! supplied by the caller (the validator's view of consensus time).

pub mod auction;
pub mod bid;
pub mod clearing;
pub mod pricing;

pub use auction::{Auction, AuctionId, AuctionStatus, Cleared, LifecycleError};
pub use bid::{Bid, BidError};
pub use clearing::{try_clear, ClearError};
pub use pricing::{current_price, PricingError};
