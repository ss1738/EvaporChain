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

#[cfg(test)]
mod press_claim_tests {
    use super::*;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "Half-Life NFT tier ladder rewards retention.
    /// Tier promotion is monotone in held-time and a transfer RESETS
    /// the retention clock to tier 0 — mercenary trading literally
    /// costs the buyer the fastest decay schedule. Tier-promotion
    /// changes the half-life only, never the energy."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let alice = [0xAAu8; 32];
        let bob = [0xBBu8; 32];

        let mut nft = HalfLifeNft::mint(
            TokenId([1u8; 32]),
            alice,
            10_000,
            0,
            default_ladder(),
        )
        .unwrap();

        // Fresh mint: tier 0 (lowest tier, fastest decay).
        assert_eq!(nft.current_tier_index(), 0);
        assert_eq!(nft.held_epochs_by_current_holder, 0);

        // Tick forward enough to cross at least one tier threshold.
        nft.tick_to(10_000).unwrap();
        let energy_after_long_hold = nft.energy;
        let tier_after_long_hold = nft.current_tier_index();
        assert!(tier_after_long_hold >= 1, "long hold must have promoted at least 1 tier");

        // Transfer resets retention clock and tier — held_epochs goes
        // back to 0 → tier 0 — but energy is preserved.
        nft.transfer(bob).unwrap();
        assert_eq!(nft.holder, bob);
        assert_eq!(nft.held_epochs_by_current_holder, 0);
        assert_eq!(nft.current_tier_index(), 0);
        assert_eq!(
            nft.energy, energy_after_long_hold,
            "transfer must NOT change energy"
        );

        // Same-holder transfer rejected.
        assert!(matches!(
            nft.transfer(bob),
            Err(NftError::TransferToSameHolder)
        ));

        // Non-monotone tick rejected.
        nft.tick_to(11_000).unwrap();
        assert!(matches!(
            nft.tick_to(10_500),
            Err(NftError::NonMonotoneTick { .. })
        ));

        // Zero initial energy rejected at mint.
        assert!(matches!(
            HalfLifeNft::mint(TokenId([2u8; 32]), alice, 0, 0, default_ladder()),
            Err(NftError::ZeroInitialEnergy)
        ));
    }
}
