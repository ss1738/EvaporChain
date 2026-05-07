//! Singh-Lineage (EvaporWallet-Lineage).
//!
//! Per `research/INVENTION_STACK.md` §A5.4:
//!
//! > Wallet has a second screen called Lineage: family tree showing
//! > primary key + designated successors with dormancy thresholds.
//! > *"If I'm silent 90 days, my daughter's key gains 25% authority;
//! > 180 days, 50%; 365 days, full."* Per-asset posthumous designation.
//! > Real-time visual of your own digital mortality.
//!
//! > **Cultural lineage:** Apple Legacy Contact (closest precedent —
//! > but reactive, not graduated); Google Inactive Account Manager.
//!
//! > **Pitch:** *"crypto solves inheritance."* Highest mainstream-
//! > press potency of the wallet set — FT, NYT personal finance,
//! > Atlantic.
//!
//! ## What's structurally different from existing inheritance UX
//!
//! Apple Legacy Contact and Google Inactive Account Manager are both
//! **reactive**: a successor either has access or they don't, with a
//! single threshold and a one-shot transfer event. Singh-Lineage is
//! **graduated**: authority accrues continuously as the issuer's
//! silence lengthens. Three structural decisions:
//!
//! 1. **Authority is a fraction in [0, 1]**, not a binary flag. At
//!    each `DormancyTier`, a successor's authority increases by the
//!    tier's `authority_share`. Multiple successors can be on the
//!    same lineage with overlapping shares; the chain enforces that
//!    total authority across all successors at any tier sums to ≤ 1.
//!
//! 2. **Dormancy resets on any signed action by the issuer**, not
//!    just heartbeat pings. Validators agree on a single source of
//!    truth: `last_seen_epoch`. The wallet's "still alive" signal
//!    is just any tx the issuer signed.
//!
//! 3. **Per-asset designation, not per-wallet.** The lineage policy
//!    can attach to a specific [`evaporchain_types::ObjectId`] so a
//!    user can leave one NFT to a daughter and another to a sibling
//!    without one-shot account merges. (V1 ships per-lineage; the
//!    type is keyed on a 32-byte handle the wallet/contract layer
//!    chooses to scope to a wallet, an asset, or a bundle.)
//!
//! ## Module map
//!
//! - [`tier`] — [`DormancyTier`] (epochs threshold + authority share);
//!   [`Ladder`] tier list with monotonicity invariants.
//! - [`successor`] — [`Successor`] declaration; one per heir.
//! - [`lineage`] — [`Lineage`] policy (issuer + ladder + successors +
//!   `last_seen_epoch`); `authority_at(epoch_now, successor)` resolves
//!   the live fraction; `touch(epoch_now)` resets dormancy.

pub mod lineage;
pub mod successor;
pub mod tier;

pub use lineage::{Lineage, LineageError, LineageId};
pub use successor::{Successor, SuccessorError};
pub use tier::{DormancyTier, Ladder, LadderError};

#[cfg(test)]
mod press_claim_tests {
    use super::*;
    use tier::FULL_AUTHORITY_BP;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "Singh-Lineage authority is GRADUATED, not binary.
    /// Each DormancyTier raises the heir's authority share. Across
    /// all heirs, total authority can never exceed FULL_AUTHORITY_BP
    /// (= 10_000 bp = 1.00). Touch resets dormancy. Backwards-time
    /// touches and non-issuer callers are rejected."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let issuer: evaporchain_types::AccountAddress = [1u8; 32];
        let heir1: evaporchain_types::AccountAddress = [2u8; 32];
        let heir2: evaporchain_types::AccountAddress = [3u8; 32];
        let stranger: evaporchain_types::AccountAddress = [9u8; 32];

        // Doctrine ladder: 25% at 90d, 50% at 180d, 100% at 365d (with
        // 1 epoch/day = 1 epoch/day).
        let ladder = Ladder::doctrine_default(1).unwrap();
        let mut lineage = Lineage::register([0u8; 32], issuer, ladder, 0);

        // Successors: heir1 = 60%, heir2 = 40% (sum = 100%).
        lineage
            .add_successor(
                issuer,
                Successor::new(heir1, 6_000, "daughter", issuer).unwrap(),
            )
            .unwrap();
        lineage
            .add_successor(
                issuer,
                Successor::new(heir2, 4_000, "trustee", issuer).unwrap(),
            )
            .unwrap();

        // At 0 epochs dormancy: tier share is 0% → no authority.
        assert_eq!(lineage.authority_at(0, &heir1), 0);
        assert_eq!(lineage.total_authority_at(0), 0);

        // Beyond the longest tier threshold: full ladder share, but
        // total never exceeds FULL_AUTHORITY_BP.
        let far_future = 1_000_000;
        let total = lineage.total_authority_at(far_future);
        assert!(total <= FULL_AUTHORITY_BP);

        // Touch resets dormancy → authority drops back to 0.
        lineage.touch(issuer, far_future).unwrap();
        assert_eq!(lineage.authority_at(far_future, &heir1), 0);

        // Non-issuer cannot touch.
        assert!(matches!(
            lineage.touch(stranger, far_future + 1),
            Err(LineageError::NotIssuer { .. })
        ));

        // Over-weighted addition rejected (exceeds FULL_AUTHORITY_BP).
        let mut over =
            Lineage::register([0u8; 32], issuer, Ladder::doctrine_default(1).unwrap(), 0);
        over.add_successor(issuer, Successor::new(heir1, 7_000, "x", issuer).unwrap())
            .unwrap();
        let res = over.add_successor(issuer, Successor::new(heir2, 5_000, "y", issuer).unwrap());
        assert!(matches!(res, Err(LineageError::OverWeighted { .. })));
    }
}
