//! Half-Life NFT with retention tiers.
//!
//! Standard "decay NFT" pattern: token energy halves every
//! `half_life` epochs. The retention-tier extension: holders who
//! keep the token across tier-promotion thresholds get a slower
//! half-life. Five-tier ladder (configurable) where each tier
//! roughly doubles the half-life. Transferring resets the
//! retention clock to tier 0 — mercenary trading literally costs
//! the buyer half the lifetime per cycle.
//!
//! ## Three structural decisions enforced as tests
//!
//! 1. **Tier promotion is monotone in held-time.** Holder cannot
//!    skip tiers; promotion happens deterministically once the
//!    cumulative held-time crosses each threshold.
//!
//! 2. **Transfer resets the retention clock.** New holder starts
//!    at tier 0. Same NFT, but its decay schedule is now the
//!    fastest. This is the "mercenary cost" mechanism.
//!
//! 3. **Tier-promotion changes the half-life only — never the
//!    energy.** Promotion is a rate change, not a rebate. The
//!    chain doesn't print energy on promotion; it just decays
//!    you slower from now on.
//!
//! ## Module map
//!
//! - [`tier`] — [`Tier`] + the default ladder.
//! - [`token`] — [`HalfLifeNft`] mint / transfer / decay-tick /
//!   read-energy.

pub mod tier;
pub mod token;

pub use tier::{Tier, TierLadder, default_ladder};
pub use token::{HalfLifeNft, NftError, TokenId};
