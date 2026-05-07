//! `Lineage` — the policy + live state.
//!
//! Composes `Ladder` (graduated thresholds) + `Successor` list
//! (heirs with weights) + `last_seen_epoch` (the issuer's "still
//! alive" signal). Validators agree on these inputs; the chain
//! computes per-successor authority pure-function at any epoch.
//!
//! Lifecycle:
//! - `register(issuer, ladder, last_seen)` — create a lineage with
//!   no successors yet.
//! - `add_successor` / `remove_successor` — issuer manages heirs.
//!   Total weight across all successors must be ≤ FULL_AUTHORITY_BP.
//! - `touch(epoch_now)` — issuer signals they're alive; resets
//!   dormancy. Caller must be the issuer.
//! - `authority_at(epoch_now, successor_address)` — pure-function
//!   query. Returns `effective_authority_bp` for that successor at
//!   that epoch.

use evaporchain_types::{AccountAddress, Epoch};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::successor::{Successor, SuccessorError};
use crate::tier::{Ladder, FULL_AUTHORITY_BP};

pub type LineageId = [u8; 32];

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LineageError {
    #[error("caller {caller:?} is not the issuer {issuer:?}")]
    NotIssuer {
        caller: AccountAddress,
        issuer: AccountAddress,
    },
    #[error("successor address {0:?} is already registered on this lineage")]
    DuplicateSuccessor(AccountAddress),
    #[error("successor address {0:?} not registered")]
    UnknownSuccessor(AccountAddress),
    #[error(
        "total successor weight {total_bp} would exceed FULL_AUTHORITY_BP ({FULL_AUTHORITY_BP})"
    )]
    OverWeighted { total_bp: u64 },
    #[error("touch epoch {now} is before last_seen {last_seen}")]
    TouchBackwardsInTime { now: Epoch, last_seen: Epoch },
    #[error("successor: {0}")]
    Successor(#[from] SuccessorError),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lineage {
    pub id: LineageId,
    pub issuer: AccountAddress,
    pub ladder: Ladder,
    pub successors: Vec<Successor>,
    /// Last epoch the issuer demonstrated they're alive (any signed
    /// `touch` or — at higher layers — any signed tx).
    pub last_seen_epoch: Epoch,
    pub registered_at_epoch: Epoch,
}

impl Lineage {
    /// Register a new lineage. Starts with no successors; `touch`
    /// initialises `last_seen_epoch` to the registration epoch.
    pub fn register(
        id: LineageId,
        issuer: AccountAddress,
        ladder: Ladder,
        registered_at_epoch: Epoch,
    ) -> Self {
        Self {
            id,
            issuer,
            ladder,
            successors: Vec::new(),
            last_seen_epoch: registered_at_epoch,
            registered_at_epoch,
        }
    }

    /// Add a successor. Issuer-gated. Rejects duplicates and
    /// over-weight totals.
    pub fn add_successor(
        &mut self,
        caller: AccountAddress,
        successor: Successor,
    ) -> Result<(), LineageError> {
        if caller != self.issuer {
            return Err(LineageError::NotIssuer {
                caller,
                issuer: self.issuer,
            });
        }
        if self
            .successors
            .iter()
            .any(|s| s.address == successor.address)
        {
            return Err(LineageError::DuplicateSuccessor(successor.address));
        }
        let new_total: u64 = self
            .successors
            .iter()
            .map(|s| s.weight_bp as u64)
            .sum::<u64>()
            + successor.weight_bp as u64;
        if new_total > FULL_AUTHORITY_BP as u64 {
            return Err(LineageError::OverWeighted {
                total_bp: new_total,
            });
        }
        self.successors.push(successor);
        Ok(())
    }

    /// Remove a successor. Issuer-gated.
    pub fn remove_successor(
        &mut self,
        caller: AccountAddress,
        successor_address: AccountAddress,
    ) -> Result<(), LineageError> {
        if caller != self.issuer {
            return Err(LineageError::NotIssuer {
                caller,
                issuer: self.issuer,
            });
        }
        let len_before = self.successors.len();
        self.successors.retain(|s| s.address != successor_address);
        if self.successors.len() == len_before {
            return Err(LineageError::UnknownSuccessor(successor_address));
        }
        Ok(())
    }

    /// Issuer signals they're alive. Resets `last_seen_epoch`. Errors
    /// on backward time and non-issuer caller.
    pub fn touch(&mut self, caller: AccountAddress, epoch_now: Epoch) -> Result<(), LineageError> {
        if caller != self.issuer {
            return Err(LineageError::NotIssuer {
                caller,
                issuer: self.issuer,
            });
        }
        if epoch_now < self.last_seen_epoch {
            return Err(LineageError::TouchBackwardsInTime {
                now: epoch_now,
                last_seen: self.last_seen_epoch,
            });
        }
        self.last_seen_epoch = epoch_now;
        Ok(())
    }

    /// Effective authority for `successor_address` at `epoch_now`,
    /// in basis points. Pure function; deterministic; integer-only.
    ///
    /// `effective_bp = ladder.share_at(dormancy) × successor.weight_bp / FULL_AUTHORITY_BP`.
    pub fn authority_at(&self, epoch_now: Epoch, successor_address: &AccountAddress) -> u32 {
        let s = match self
            .successors
            .iter()
            .find(|s| &s.address == successor_address)
        {
            Some(s) => s,
            None => return 0,
        };
        let dormancy = epoch_now.saturating_sub(self.last_seen_epoch);
        let tier_share = self.ladder.share_at(dormancy);
        // (tier_share × weight) / FULL — integer math via u64.
        let eff = (tier_share as u64 * s.weight_bp as u64) / FULL_AUTHORITY_BP as u64;
        eff.min(FULL_AUTHORITY_BP as u64) as u32
    }

    /// Total live authority across all successors at `epoch_now`. The
    /// invariant the doctrine cares about: no matter how dormant the
    /// issuer becomes, total authority across heirs cannot exceed
    /// FULL_AUTHORITY_BP. Useful for wallets to render "100% of my
    /// inheritance is allocated."
    pub fn total_authority_at(&self, epoch_now: Epoch) -> u32 {
        let dormancy = epoch_now.saturating_sub(self.last_seen_epoch);
        let tier_share = self.ladder.share_at(dormancy) as u64;
        let total_weight: u64 = self
            .successors
            .iter()
            .map(|s| s.weight_bp as u64)
            .sum::<u64>();
        let eff = (tier_share * total_weight) / FULL_AUTHORITY_BP as u64;
        eff.min(FULL_AUTHORITY_BP as u64) as u32
    }

    /// Dormancy (epochs since `last_seen`). Useful for the wallet's
    /// "real-time visual of your own digital mortality" countdown.
    pub fn dormancy_epochs(&self, epoch_now: Epoch) -> u64 {
        epoch_now.saturating_sub(self.last_seen_epoch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tier::Ladder;

    fn id(b: u8) -> LineageId {
        let mut x = [0u8; 32];
        x[0] = b;
        x
    }

    fn addr(b: u8) -> AccountAddress {
        id(b)
    }

    fn doctrine_lineage() -> Lineage {
        let ladder = Ladder::doctrine_default(1).unwrap();
        Lineage::register(id(0xFF), addr(1), ladder, 0)
    }

    fn daughter() -> Successor {
        Successor::new(addr(2), 10_000, "daughter", addr(1)).unwrap()
    }

    fn son() -> Successor {
        Successor::new(addr(3), 5_000, "son", addr(1)).unwrap()
    }

    // ─── Successor management ─────────────────────────────────────────

    #[test]
    fn add_successor_requires_issuer() {
        let mut l = doctrine_lineage();
        let err = l.add_successor(addr(2), daughter()).unwrap_err();
        assert!(matches!(err, LineageError::NotIssuer { .. }));
    }

    #[test]
    fn add_duplicate_successor_rejected() {
        let mut l = doctrine_lineage();
        l.add_successor(addr(1), daughter()).unwrap();
        let err = l.add_successor(addr(1), daughter()).unwrap_err();
        assert!(matches!(err, LineageError::DuplicateSuccessor(_)));
    }

    #[test]
    fn over_weighted_lineage_rejected() {
        let mut l = doctrine_lineage();
        l.add_successor(addr(1), daughter()).unwrap(); // 10000bp
                                                       // Adding a son with 5000bp would push total to 15000 > 10000.
        let err = l.add_successor(addr(1), son()).unwrap_err();
        assert!(matches!(err, LineageError::OverWeighted { .. }));
    }

    #[test]
    fn balanced_two_successors_valid() {
        let mut l = doctrine_lineage();
        // Two children, 50% each.
        let c1 = Successor::new(addr(2), 5_000, "child-1", addr(1)).unwrap();
        let c2 = Successor::new(addr(3), 5_000, "child-2", addr(1)).unwrap();
        l.add_successor(addr(1), c1).unwrap();
        l.add_successor(addr(1), c2).unwrap();
        assert_eq!(l.successors.len(), 2);
    }

    #[test]
    fn remove_successor_works() {
        let mut l = doctrine_lineage();
        l.add_successor(addr(1), daughter()).unwrap();
        l.remove_successor(addr(1), addr(2)).unwrap();
        assert!(l.successors.is_empty());
    }

    #[test]
    fn remove_nonexistent_successor_errors() {
        let mut l = doctrine_lineage();
        let err = l.remove_successor(addr(1), addr(99)).unwrap_err();
        assert!(matches!(err, LineageError::UnknownSuccessor(_)));
    }

    // ─── Touch (heartbeat) ─────────────────────────────────────────

    #[test]
    fn touch_requires_issuer() {
        let mut l = doctrine_lineage();
        let err = l.touch(addr(2), 100).unwrap_err();
        assert!(matches!(err, LineageError::NotIssuer { .. }));
    }

    #[test]
    fn touch_advances_last_seen() {
        let mut l = doctrine_lineage();
        l.touch(addr(1), 50).unwrap();
        assert_eq!(l.last_seen_epoch, 50);
        l.touch(addr(1), 200).unwrap();
        assert_eq!(l.last_seen_epoch, 200);
    }

    #[test]
    fn touch_backwards_in_time_rejected() {
        let mut l = doctrine_lineage();
        l.touch(addr(1), 100).unwrap();
        let err = l.touch(addr(1), 50).unwrap_err();
        assert!(matches!(err, LineageError::TouchBackwardsInTime { .. }));
    }

    // ─── Authority queries ─────────────────────────────────────────

    #[test]
    fn authority_zero_before_first_tier() {
        let mut l = doctrine_lineage();
        l.add_successor(addr(1), daughter()).unwrap();
        // At epoch 50 (issuer last seen at 0; ladder first tier is 90),
        // dormancy = 50, below first threshold ⇒ zero authority.
        assert_eq!(l.authority_at(50, &addr(2)), 0);
    }

    #[test]
    fn authority_steps_at_each_tier_per_doctrine() {
        // Doctrine quote: "If I'm silent 90 days, my daughter's key
        // gains 25% authority; 180 days, 50%; 365 days, full."
        let mut l = doctrine_lineage();
        l.add_successor(addr(1), daughter()).unwrap();
        assert_eq!(l.authority_at(89, &addr(2)), 0);
        assert_eq!(l.authority_at(90, &addr(2)), 2_500); // 25%
        assert_eq!(l.authority_at(180, &addr(2)), 5_000); // 50%
        assert_eq!(l.authority_at(365, &addr(2)), 10_000); // full
        assert_eq!(l.authority_at(99_999, &addr(2)), 10_000);
    }

    #[test]
    fn authority_for_unknown_successor_is_zero() {
        let l = doctrine_lineage();
        assert_eq!(l.authority_at(1000, &addr(99)), 0);
    }

    #[test]
    fn split_inheritance_distributes_share() {
        // Two children, 50% weight each. At full dormancy, each gets
        // 50% of the ladder's 100% share = 5_000 bp each.
        let mut l = doctrine_lineage();
        let c1 = Successor::new(addr(2), 5_000, "c1", addr(1)).unwrap();
        let c2 = Successor::new(addr(3), 5_000, "c2", addr(1)).unwrap();
        l.add_successor(addr(1), c1).unwrap();
        l.add_successor(addr(1), c2).unwrap();
        assert_eq!(l.authority_at(365, &addr(2)), 5_000);
        assert_eq!(l.authority_at(365, &addr(3)), 5_000);
        // Total cannot exceed full authority.
        assert_eq!(l.total_authority_at(365), 10_000);
    }

    #[test]
    fn touch_resets_authority() {
        // Doctrine: "dormancy resets on any signed action by the
        // issuer." Touch at high dormancy snaps successor's authority
        // back to zero.
        let mut l = doctrine_lineage();
        l.add_successor(addr(1), daughter()).unwrap();
        // At epoch 200, daughter has 50% authority.
        assert_eq!(l.authority_at(200, &addr(2)), 5_000);
        // Issuer touches at epoch 200: resets last_seen.
        l.touch(addr(1), 200).unwrap();
        // At epoch 200, dormancy = 0 ⇒ zero authority.
        assert_eq!(l.authority_at(200, &addr(2)), 0);
        // At epoch 290, dormancy = 90 ⇒ 25% (not 50%, because the
        // clock reset).
        assert_eq!(l.authority_at(290, &addr(2)), 2_500);
    }

    // ─── Press claim ─────────────────────────────────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // "Crypto solves inheritance." Operationalised:
        // 1. Living issuer: heirs have zero authority.
        // 2. As silence lengthens, authority accrues *gradually*.
        // 3. At full dormancy, total authority across heirs sums to
        //    exactly FULL_AUTHORITY_BP — no double-spend, no
        //    fractional loss.
        // 4. Any heartbeat from the issuer resets the clock.
        let mut l = doctrine_lineage();
        let c1 = Successor::new(addr(2), 5_000, "daughter", addr(1)).unwrap();
        let c2 = Successor::new(addr(3), 5_000, "son", addr(1)).unwrap();
        l.add_successor(addr(1), c1).unwrap();
        l.add_successor(addr(1), c2).unwrap();

        // 1. Living: zero authority across the board.
        assert_eq!(l.total_authority_at(50), 0);
        // 2. Graduated accrual.
        assert_eq!(l.total_authority_at(90), 2_500);
        assert_eq!(l.total_authority_at(180), 5_000);
        // 3. Full dormancy: total = full authority, exactly.
        assert_eq!(l.total_authority_at(365), 10_000);
        assert_eq!(
            l.authority_at(365, &addr(2)) + l.authority_at(365, &addr(3)),
            10_000
        );
        // 4. Heartbeat resets.
        l.touch(addr(1), 365).unwrap();
        assert_eq!(l.total_authority_at(365), 0);
    }

    #[test]
    fn dormancy_epochs_returns_silence_duration() {
        let mut l = doctrine_lineage();
        l.touch(addr(1), 100).unwrap();
        assert_eq!(l.dormancy_epochs(100), 0);
        assert_eq!(l.dormancy_epochs(150), 50);
        assert_eq!(l.dormancy_epochs(50), 0); // saturating; clock-skew defensive
    }

    #[test]
    fn round_trip_serde_full_lifecycle() {
        let mut l = doctrine_lineage();
        l.add_successor(addr(1), daughter()).unwrap();
        l.touch(addr(1), 50).unwrap();
        let s = serde_json::to_string(&l).unwrap();
        let back: Lineage = serde_json::from_str(&s).unwrap();
        assert_eq!(l, back);
    }
}
