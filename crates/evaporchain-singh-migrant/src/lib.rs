//! Singh-Migrant (Wanderwrits) — NFTs that die if held still.
//!
//! Per `research/INVENTION_STACK.md` §A5.3:
//!
//! > Each NFT has *resting threshold* (~30 days). Energy decays
//! > normally; transfer to a *new* wallet refunds a fraction. Stay
//! > still past threshold → λ doubles; past 60 days → quadruples.
//! > Must keep moving through novel hands or it evaporates.
//!
//! > **Pitch:** *"the NFT that dies if you keep it."*
//!
//! ## Three structural decisions
//!
//! 1. **Effective half-life multiplier scales with rest age.** Below
//!    `resting_threshold_epochs`, decay runs at the base half-life
//!    (multiplier 1×). Past the threshold, multiplier becomes 0.5×
//!    (twice as fast = half the half-life). Past
//!    `2 * resting_threshold_epochs`, multiplier becomes 0.25×
//!    (quadruple speed). The token's apparent decay accelerates as it
//!    sits — so "stay still" really does kill it faster.
//!
//! 2. **Transfer to a NOVEL wallet is required for refund.** Not just
//!    "any transfer." The token tracks its `visited_wallets` set
//!    on-chain. Transfer to an address already in the set is a *legal
//!    move* but yields zero refund. The kula-ring metaphor holds: the
//!    object circulates; it doesn't bounce.
//!
//! 3. **Refund is a fraction of *current* energy, not initial.** A
//!    near-dead token gets a small refund; a fresh one gets a big
//!    one. This kills the trivial farm-and-relay attack: holding it
//!    until it's nearly dead and then circulating doesn't restore
//!    much.
//!
//! Cultural lineage: Trobriand kula ring (Malinowski 1922), chain
//! letters, Olympic torch, Marcel Mauss *The Gift* (1925).
//!
//! ## Module map
//!
//! - [`decay`] — [`effective_half_life`]: piecewise multiplier;
//!   [`current_energy`]: energy at `epoch_now` given `(initial,
//!   half_life, rested_at, threshold)`.
//! - [`refund`] — [`refund_amount`]: fraction-of-current refund on
//!   novel-wallet transfer.
//! - [`token`] — [`MigrantToken`], [`TokenId`]; mint, transfer,
//!   witness.

pub mod decay;
pub mod refund;
pub mod token;

pub use decay::{current_energy, effective_half_life, DecayError};
pub use refund::{refund_amount, RefundError, REFUND_FRACTION_PCT};
pub use token::{MigrantToken, TokenError, TokenId, TransferOutcome};

#[cfg(test)]
mod press_claim_tests {
    use super::*;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "Singh-Migrant tokens DIE FASTER if held still.
    /// Tier 1 (rest < threshold): unchanged half-life. Tier 2
    /// (threshold ≤ rest < 2×threshold): half-life halved. Tier 3
    /// (rest ≥ 2×threshold): half-life quartered. Refund on novel-
    /// wallet transfer is `REFUND_FRACTION_PCT` of *current* energy
    /// — a near-dead token gets a small refund, killing the
    /// farm-and-relay attack."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Effective half-life tiers.
        assert_eq!(effective_half_life(1_000, 50, 100).unwrap(), 1_000); // tier 1
        assert_eq!(effective_half_life(1_000, 100, 100).unwrap(), 500); // tier 2
        assert_eq!(effective_half_life(1_000, 250, 100).unwrap(), 250); // tier 3

        // Tier ordering: tier-1 ≥ tier-2 ≥ tier-3.
        let h1 = effective_half_life(1_000, 50, 100).unwrap();
        let h2 = effective_half_life(1_000, 150, 100).unwrap();
        let h3 = effective_half_life(1_000, 300, 100).unwrap();
        assert!(h1 >= h2 && h2 >= h3);

        // refund_amount returns POST-refund energy = min(initial,
        // current + REFUND_FRACTION_PCT% × current). Fresh token at
        // initial caps back at initial.
        let fresh_post = refund_amount(1_000, 1_000).unwrap();
        assert_eq!(fresh_post, 1_000, "post-refund clamps at initial");

        // Mid-decayed token: current=400 → refund 100 → post 500.
        let mid_post = refund_amount(1_000, 400).unwrap();
        assert_eq!(mid_post, 400 + 400 * REFUND_FRACTION_PCT / 100);

        // Near-dead token gets proportionally small refund.
        let dying_post = refund_amount(1_000, 100).unwrap();
        assert!(dying_post < mid_post, "dying refund < mid refund");

        // Doctrine constant.
        assert_eq!(REFUND_FRACTION_PCT, 25);

        // Zero base/threshold rejected.
        assert!(matches!(
            effective_half_life(0, 10, 100),
            Err(DecayError::ZeroHalfLife)
        ));
        assert!(matches!(
            effective_half_life(1_000, 10, 0),
            Err(DecayError::ZeroThreshold)
        ));
    }
}
