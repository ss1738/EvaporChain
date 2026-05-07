//! `MigrantToken` — the on-chain Wanderwrit.
//!
//! Tracks `visited_wallets` so a transfer can be classified as either
//! novel (refund-eligible) or revisit (legal but no refund).
//! `rested_at_epoch` is reset only on novel transfers — revisits do
//! not heal the rest counter.

use evaporchain_types::{AccountAddress, Energy, Epoch, HalfLife};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use thiserror::Error;

use crate::decay::{current_energy, DecayError};
use crate::refund::{refund_amount, RefundError};

/// 32-byte opaque token handle.
pub type TokenId = [u8; 32];

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TokenError {
    #[error("initial energy must be > 0")]
    ZeroInitial,
    #[error("base_half_life must be > 0")]
    ZeroHalfLife,
    #[error("resting_threshold_epochs must be > 0")]
    ZeroThreshold,
    #[error("transfer caller {caller:?} is not the current owner {owner:?}")]
    NotOwner {
        caller: AccountAddress,
        owner: AccountAddress,
    },
    #[error("transfer to self is a no-op")]
    SelfTransfer,
    #[error("token has fully evaporated (energy=0); transfers blocked")]
    Evaporated,
    #[error("decay computation failed: {0}")]
    Decay(#[from] DecayError),
    #[error("refund computation failed: {0}")]
    Refund(#[from] RefundError),
}

/// Outcome reported when `transfer` succeeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferOutcome {
    /// Recipient was not in `visited_wallets`. Refund applied; rest
    /// counter reset to `epoch_now`.
    Novel { post_energy: Energy },
    /// Recipient was already in `visited_wallets`. Legal transfer but
    /// zero refund; rest counter not reset.
    Revisit { post_energy: Energy },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrantToken {
    pub id: TokenId,
    pub minted_by: AccountAddress,
    pub minted_at_epoch: Epoch,
    /// Initial energy at mint — also the cap on `cached_energy` after refund.
    pub initial: Energy,
    pub base_half_life: HalfLife,
    /// Resting threshold (the doctrine ~30-day knob). Past this, the
    /// effective half-life halves.
    pub resting_threshold_epochs: u64,
    pub owner: AccountAddress,
    /// Energy snapshot from the last state-changing event (mint or
    /// novel transfer). `current` energy at any later epoch is derived
    /// from `(cached_energy, base_half_life, rested_at_epoch,
    /// resting_threshold_epochs, epoch_now)`.
    pub cached_energy: Energy,
    /// Epoch the token last moved to a novel wallet. Mint counts as
    /// the first novel placement.
    pub rested_at_epoch: Epoch,
    /// Set of all wallets the token has ever resided in. Used to
    /// classify transfers as Novel vs Revisit. BTreeSet so the
    /// serialised form is canonical (validators agree on ordering).
    pub visited_wallets: BTreeSet<AccountAddress>,
}

impl MigrantToken {
    pub fn mint(
        id: TokenId,
        minted_by: AccountAddress,
        initial: Energy,
        base_half_life: HalfLife,
        resting_threshold_epochs: u64,
        minted_at_epoch: Epoch,
    ) -> Result<Self, TokenError> {
        if initial == 0 {
            return Err(TokenError::ZeroInitial);
        }
        if base_half_life == 0 {
            return Err(TokenError::ZeroHalfLife);
        }
        if resting_threshold_epochs == 0 {
            return Err(TokenError::ZeroThreshold);
        }
        let mut visited = BTreeSet::new();
        visited.insert(minted_by);
        Ok(Self {
            id,
            minted_by,
            minted_at_epoch,
            initial,
            base_half_life,
            resting_threshold_epochs,
            owner: minted_by,
            cached_energy: initial,
            rested_at_epoch: minted_at_epoch,
            visited_wallets: visited,
        })
    }

    /// Energy at `epoch_now`, computed from the cached snapshot.
    pub fn energy_at(&self, epoch_now: Epoch) -> Result<Energy, TokenError> {
        Ok(current_energy(
            self.cached_energy,
            self.base_half_life,
            self.rested_at_epoch,
            self.resting_threshold_epochs,
            epoch_now,
        )?)
    }

    /// True iff the token has fully evaporated as of `epoch_now`.
    pub fn is_evaporated(&self, epoch_now: Epoch) -> bool {
        self.energy_at(epoch_now).map(|e| e == 0).unwrap_or(false)
    }

    /// Transfer the token. Caller must be the current owner.
    /// Self-transfers are rejected (no useful state change). A
    /// transfer to a wallet already in `visited_wallets` is a *legal*
    /// move but yields no refund and no rest reset.
    pub fn transfer(
        &mut self,
        caller: AccountAddress,
        new_owner: AccountAddress,
        epoch_now: Epoch,
    ) -> Result<TransferOutcome, TokenError> {
        if caller != self.owner {
            return Err(TokenError::NotOwner {
                caller,
                owner: self.owner,
            });
        }
        if new_owner == self.owner {
            return Err(TokenError::SelfTransfer);
        }
        // Check current energy. If zero, the token is dead — block
        // all transfers so callers can't ferry an evaporated token.
        let energy_now = self.energy_at(epoch_now)?;
        if energy_now == 0 {
            return Err(TokenError::Evaporated);
        }
        let novel = !self.visited_wallets.contains(&new_owner);
        if novel {
            // Apply refund and reset the rest counter.
            let post = refund_amount(self.initial, energy_now)?;
            self.cached_energy = post;
            self.rested_at_epoch = epoch_now;
            self.visited_wallets.insert(new_owner);
            self.owner = new_owner;
            Ok(TransferOutcome::Novel { post_energy: post })
        } else {
            // Revisit — legal move but no refund. We *do* update the
            // cached_energy snapshot so future reads see the
            // up-to-date value, but the rest counter is preserved
            // (the token is still "resting" from its last novel move).
            let rest_unchanged = self.rested_at_epoch;
            self.cached_energy = energy_now;
            // Re-anchor the cache to epoch_now so future decay is
            // measured from here, but preserve the original rested_at
            // so the *tier* doesn't change. Achieved by making
            // rested_at_epoch unchanged and just letting the cache
            // be the energy as-of-now (math still works since we're
            // anchoring "current snapshot" + "rest from rested_at").
            // Subtle: keep rested_at_epoch untouched.
            self.rested_at_epoch = rest_unchanged;
            self.owner = new_owner;
            Ok(TransferOutcome::Revisit {
                post_energy: energy_now,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(b: u8) -> TokenId {
        let mut x = [0u8; 32];
        x[0] = b;
        x
    }

    fn addr(b: u8) -> AccountAddress {
        id(b)
    }

    fn fresh(creator: u8) -> MigrantToken {
        MigrantToken::mint(id(0xFF), addr(creator), 1000, 100, 30, 0).unwrap()
    }

    #[test]
    fn mint_rejects_zero_initial() {
        let err = MigrantToken::mint(id(1), addr(0xAA), 0, 100, 30, 0).unwrap_err();
        assert_eq!(err, TokenError::ZeroInitial);
    }

    #[test]
    fn mint_rejects_zero_half_life() {
        let err = MigrantToken::mint(id(1), addr(0xAA), 1000, 0, 30, 0).unwrap_err();
        assert_eq!(err, TokenError::ZeroHalfLife);
    }

    #[test]
    fn mint_rejects_zero_threshold() {
        let err = MigrantToken::mint(id(1), addr(0xAA), 1000, 100, 0, 0).unwrap_err();
        assert_eq!(err, TokenError::ZeroThreshold);
    }

    #[test]
    fn mint_records_creator_in_visited_set() {
        let t = fresh(0xAA);
        assert_eq!(t.owner, addr(0xAA));
        assert!(t.visited_wallets.contains(&addr(0xAA)));
        assert_eq!(t.visited_wallets.len(), 1);
    }

    #[test]
    fn transfer_requires_current_owner() {
        let mut t = fresh(0xAA);
        let err = t.transfer(addr(0xBB), addr(0xCC), 10).unwrap_err();
        assert!(matches!(err, TokenError::NotOwner { .. }));
    }

    #[test]
    fn transfer_to_self_rejected() {
        let mut t = fresh(0xAA);
        let err = t.transfer(addr(0xAA), addr(0xAA), 10).unwrap_err();
        assert_eq!(err, TokenError::SelfTransfer);
    }

    #[test]
    fn novel_transfer_resets_rest_and_refunds() {
        // base half-life=100, threshold=30. Sit for 50 epochs (in tier 2:
        // h_eff=50). At t=50, energy=current(1000, 100, 0, 30, 50). Tier 2
        // ⇒ h_eff=50, energy = 1000 / 2^(50/50) = 500. Refund 25% ⇒ 625.
        let mut t = fresh(0xAA);
        let outcome = t.transfer(addr(0xAA), addr(0xBB), 50).unwrap();
        match outcome {
            TransferOutcome::Novel { post_energy } => {
                assert!(
                    post_energy > 500,
                    "expected refund > 500, got {post_energy}"
                );
                assert!(post_energy <= 1000, "must not inflate above initial");
            }
            other => panic!("expected Novel, got {other:?}"),
        }
        assert_eq!(t.owner, addr(0xBB));
        assert_eq!(t.rested_at_epoch, 50, "rest counter resets on novel");
        assert!(t.visited_wallets.contains(&addr(0xBB)));
    }

    #[test]
    fn revisit_transfer_no_refund_no_rest_reset() {
        // 0xAA → 0xBB (novel) → 0xAA (revisit; AA is in the set already).
        let mut t = fresh(0xAA);
        t.transfer(addr(0xAA), addr(0xBB), 10).unwrap();
        let energy_after_first = t.cached_energy;
        let outcome = t.transfer(addr(0xBB), addr(0xAA), 30).unwrap();
        match outcome {
            TransferOutcome::Revisit { post_energy } => {
                // post_energy reflects continued decay between epoch 10 and 30,
                // and crucially does NOT receive the 25% refund.
                assert!(
                    post_energy <= energy_after_first,
                    "revisit must not refund: post={post_energy}, prior={energy_after_first}"
                );
            }
            other => panic!("expected Revisit, got {other:?}"),
        }
        assert_eq!(t.owner, addr(0xAA));
        // visited set should NOT have grown (0xAA was already there).
        assert_eq!(t.visited_wallets.len(), 2);
    }

    #[test]
    fn fully_evaporated_token_blocks_transfer() {
        // Tiny initial energy + small half-life + huge wait ⇒ token decays to 0.
        let mut t = MigrantToken::mint(id(1), addr(0xAA), 4, 1, 30, 0).unwrap();
        // After 1000 epochs (in tier 3, h_eff=1, doubling 1000 times),
        // energy = 0.
        let err = t.transfer(addr(0xAA), addr(0xBB), 1000).unwrap_err();
        assert_eq!(err, TokenError::Evaporated);
    }

    #[test]
    fn farm_and_relay_attack_yields_diminishing_refunds() {
        // Doctrine claim: holding until near-dead and then circulating
        // doesn't restore much. Construct two scenarios with a generous
        // initial budget so neither evaporates before we can measure:
        //   (a) circulate every 10 epochs (fast — stays in tier 1 mostly)
        //   (b) circulate every 50 epochs (slow — crosses into tier 2/3)
        let mut a = MigrantToken::mint(id(1), addr(0x00), 1_000_000, 200, 30, 0).unwrap();
        let mut b = MigrantToken::mint(id(2), addr(0x00), 1_000_000, 200, 30, 0).unwrap();
        // Three distinct hops at 10-epoch intervals (tier 1 throughout).
        let mut owner_a = addr(0x00);
        for hop in 1..=3 {
            let next = addr(hop as u8 + 0x10);
            a.transfer(owner_a, next, hop as u64 * 10).unwrap();
            owner_a = next;
        }
        // Three distinct hops at 50-epoch intervals (each rest crosses the
        // threshold, dropping into tier 2 each time).
        let mut owner_b = addr(0x00);
        for hop in 1..=3 {
            let next = addr(hop as u8 + 0x20);
            b.transfer(owner_b, next, hop as u64 * 50).unwrap();
            owner_b = next;
        }
        // Fast circulation preserves more residual energy than slow.
        assert!(
            a.cached_energy > b.cached_energy,
            "fast a={} should be > slow b={}",
            a.cached_energy,
            b.cached_energy
        );
    }

    #[test]
    fn round_trip_serde() {
        let mut t = fresh(0xAA);
        t.transfer(addr(0xAA), addr(0xBB), 5).unwrap();
        let s = serde_json::to_string(&t).unwrap();
        let back: MigrantToken = serde_json::from_str(&s).unwrap();
        assert_eq!(t, back);
    }
}
