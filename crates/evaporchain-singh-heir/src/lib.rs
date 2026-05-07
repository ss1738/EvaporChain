//! Singh-Heir (Patrilithic Tokens) — kin-graph heirloom NFTs.
//!
//! ## What this is
//!
//! Holders declare an **ordered kin list** (preferred heirs in
//! priority order). On certified death of the holder, the token
//! transfers to the highest-ranked living heir who is still
//! alive and has not refused inheritance.
//!
//! Each inheritance hop applies an **inheritance-tax half-life**
//! to the token's energy: `new_energy = old_energy / 2`. Across
//! generations, the heirloom value decays geometrically unless
//! heirs actively reinforce (refresh) the energy. The chain's
//! single-λ binds heirloom longevity to active engagement.
//!
//! ## Three structural decisions enforced as tests
//!
//! 1. **Heirs ordered, deterministic.** The kin list is a Vec
//!    in priority order. Validators agree on the next heir.
//!
//! 2. **Inheritance halves energy.** No exceptions. Heirs
//!    cannot bypass the tax; they can only refresh AFTER they
//!    inherit.
//!
//! 3. **Liveness gate on heirs.** A dead-or-decayed heir is
//!    skipped (the chain holds the certificates). Inheritance
//!    walks down the priority list until it finds a live
//!    candidate. If none live, the token is *escheated*
//!    (returned to the chain's commons pool).
//!
//! ## What this crate does NOT do
//!
//! - Does NOT verify the death certificate. Caller passes a
//!    signed cert; chain's higher layer verifies the m-of-n
//!    threshold attestation.
//! - Does NOT enforce blood / adoption distinctions. The kin
//!    list is opaque labels; the chain's higher layer can
//!    classify externally.
//! - Does NOT model joint inheritance (multiple heirs splitting).
//!    V1 single-heir transfer; V2 multi-share splits.
//!
//! ## Module map
//!
//! - [`token`] — [`HeirloomNft`] state machine.

pub mod token;

pub use token::{HeirloomError, HeirloomNft, TokenId};

#[cfg(test)]
mod press_claim_tests {
    use super::*;
    use token::{HeirState, KinEdge};

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "Singh-Heir applies a HARD inheritance-tax
    /// half-life: each inheritance hop halves the token's energy.
    /// Inheritance walks the kin list in priority order, skipping
    /// Dead/Refused heirs. If no live heir exists, the token
    /// escheats (returns to commons). Inheriting requires the
    /// holder to be certified dead first."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let alice = [0xAAu8; 32];
        let bob = [0xBBu8; 32];
        let carol = [0xCCu8; 32];
        let dave = [0xDDu8; 32];

        // Mint with 3 heirs: bob (Live), carol (Dead), dave (Refused).
        // Inheritance must skip carol+dave and land on bob.
        // Wait — bob is Live so that's the path. Let me also test that
        // priority-0 dead heirs are skipped.
        let kin = vec![
            KinEdge {
                heir: bob,
                state: HeirState::Dead,
            },
            KinEdge {
                heir: carol,
                state: HeirState::Live,
            },
            KinEdge {
                heir: dave,
                state: HeirState::Refused,
            },
        ];
        let mut nft = HeirloomNft::mint(TokenId([1; 32]), alice, 1_000, kin, 0).unwrap();
        assert_eq!(nft.energy, 1_000);
        assert_eq!(nft.generation, 0);

        // Cannot inherit while holder still alive.
        assert!(matches!(
            nft.inherit(),
            Err(HeirloomError::HolderStillAlive)
        ));

        // Certify holder death; now inheritance proceeds.
        nft.certify_holder_death().unwrap();
        let new_holder = nft.inherit().unwrap();

        // Highest-priority LIVE heir is carol (bob is Dead).
        assert_eq!(new_holder, carol);
        // Energy halved (1000 → 500).
        assert_eq!(nft.energy, 500);
        // Generation bumped.
        assert_eq!(nft.generation, 1);
        // Kin list cleared for the new holder.
        assert!(nft.kin.is_empty());

        // Self-loop kin is rejected at mint (founder can't list self).
        let bad_kin = vec![KinEdge {
            heir: alice,
            state: HeirState::Live,
        }];
        assert!(matches!(
            HeirloomNft::mint(TokenId([2; 32]), alice, 1_000, bad_kin, 0),
            Err(HeirloomError::SelfLoopKin)
        ));

        // No live heirs → escheat.
        let dead_kin = vec![
            KinEdge {
                heir: bob,
                state: HeirState::Dead,
            },
            KinEdge {
                heir: carol,
                state: HeirState::Refused,
            },
        ];
        let mut nft2 = HeirloomNft::mint(TokenId([3; 32]), alice, 1_000, dead_kin, 0).unwrap();
        nft2.certify_holder_death().unwrap();
        assert!(matches!(nft2.inherit(), Err(HeirloomError::NoLiveHeirs)));
        assert!(nft2.escheated);
    }
}
