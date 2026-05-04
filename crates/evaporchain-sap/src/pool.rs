//! `AttentionPool` — per-wallet AQ minting with rate cap.
//!
//! Validators agree on the rate-limited mint outcome: a wallet
//! cannot mint more than `max_aq_per_window` quanta in a rolling
//! `window_epochs` window. This is the anti-Sybil + anti-flood
//! guardrail.
//!
//! V1 model: the pool tracks every AQ minted so far for each holder.
//! When `mint_aq(holder, ...)` is called, we count how many of that
//! holder's AQs have `minted_at_epoch >= epoch_now - window_epochs`
//! and reject if the count is at cap. The AQ list is the audit
//! trail.

use evaporchain_types::{AccountAddress, Energy, Epoch, HalfLife};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

use crate::quantum::{AqId, AqMintError, AttentionQuantum};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PoolError {
    #[error("max_aq_per_window must be > 0")]
    ZeroMaxAq,
    #[error("window_epochs must be > 0")]
    ZeroWindow,
    #[error(
        "wallet {wallet:?} has hit the rate cap: {minted_in_window} of {cap} in window"
    )]
    RateCapped {
        wallet: AccountAddress,
        minted_in_window: u64,
        cap: u64,
    },
    #[error("aq id {0:?} already exists in pool")]
    DuplicateAqId(AqId),
    #[error("backwards-time mint: epoch {now} < last_mint {last}")]
    BackwardsTime { now: Epoch, last: Epoch },
    #[error("aq mint: {0}")]
    AqMint(#[from] AqMintError),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttentionPool {
    /// Maximum AQs a single wallet may mint per `window_epochs`.
    pub max_aq_per_window: u64,
    pub window_epochs: u64,
    /// All AQs ever minted, keyed by id (canonical for serde).
    pub quanta: HashMap<AqId, AttentionQuantum>,
    /// Tracks the most-recent mint epoch *across all wallets* for a
    /// monotone-time guard. Per-wallet would be more permissive but
    /// complicates the audit trail; one global cursor is the V1
    /// simplification.
    pub last_mint_epoch: Epoch,
}

impl AttentionPool {
    pub fn new(max_aq_per_window: u64, window_epochs: u64) -> Result<Self, PoolError> {
        if max_aq_per_window == 0 {
            return Err(PoolError::ZeroMaxAq);
        }
        if window_epochs == 0 {
            return Err(PoolError::ZeroWindow);
        }
        Ok(Self {
            max_aq_per_window,
            window_epochs,
            quanta: HashMap::new(),
            last_mint_epoch: 0,
        })
    }

    /// Count AQs for `holder` minted within the last `window_epochs`
    /// before `epoch_now`. Pure read; used by the cap check.
    pub fn minted_in_window(&self, holder: &AccountAddress, epoch_now: Epoch) -> u64 {
        let cutoff = epoch_now.saturating_sub(self.window_epochs);
        self.quanta
            .values()
            .filter(|aq| &aq.holder == holder && aq.minted_at_epoch >= cutoff)
            .count() as u64
    }

    /// Mint a new AQ if the holder is under their rate cap.
    pub fn mint_aq(
        &mut self,
        id: AqId,
        holder: AccountAddress,
        initial_value: Energy,
        half_life: HalfLife,
        epoch_now: Epoch,
    ) -> Result<&AttentionQuantum, PoolError> {
        if epoch_now < self.last_mint_epoch {
            return Err(PoolError::BackwardsTime {
                now: epoch_now,
                last: self.last_mint_epoch,
            });
        }
        if self.quanta.contains_key(&id) {
            return Err(PoolError::DuplicateAqId(id));
        }
        let in_window = self.minted_in_window(&holder, epoch_now);
        if in_window >= self.max_aq_per_window {
            return Err(PoolError::RateCapped {
                wallet: holder,
                minted_in_window: in_window,
                cap: self.max_aq_per_window,
            });
        }
        let aq = AttentionQuantum::mint(id, holder, initial_value, half_life, epoch_now)?;
        self.quanta.insert(id, aq);
        self.last_mint_epoch = epoch_now;
        Ok(self.quanta.get(&id).unwrap())
    }

    /// All AQs currently in the pool for a given holder.
    pub fn aqs_for(&self, holder: &AccountAddress) -> Vec<&AttentionQuantum> {
        self.quanta
            .values()
            .filter(|aq| &aq.holder == holder)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(b: u8) -> AqId {
        let mut x = [0u8; 32];
        x[0] = b;
        x
    }

    fn addr(b: u8) -> AccountAddress {
        id(b)
    }

    #[test]
    fn rejects_zero_cap() {
        assert_eq!(
            AttentionPool::new(0, 60).unwrap_err(),
            PoolError::ZeroMaxAq
        );
    }

    #[test]
    fn rejects_zero_window() {
        assert_eq!(
            AttentionPool::new(5, 0).unwrap_err(),
            PoolError::ZeroWindow
        );
    }

    #[test]
    fn first_mint_succeeds_with_aq_recorded() {
        let mut pool = AttentionPool::new(5, 60).unwrap();
        let aq = pool.mint_aq(id(1), addr(0xAA), 1000, 45, 0).unwrap();
        assert_eq!(aq.initial_value, 1000);
        assert_eq!(pool.quanta.len(), 1);
    }

    #[test]
    fn duplicate_aq_id_rejected() {
        let mut pool = AttentionPool::new(5, 60).unwrap();
        pool.mint_aq(id(1), addr(0xAA), 1000, 45, 0).unwrap();
        let err = pool.mint_aq(id(1), addr(0xAA), 500, 45, 1).unwrap_err();
        assert_eq!(err, PoolError::DuplicateAqId(id(1)));
    }

    #[test]
    fn backwards_time_rejected() {
        let mut pool = AttentionPool::new(5, 60).unwrap();
        pool.mint_aq(id(1), addr(0xAA), 1000, 45, 100).unwrap();
        let err = pool.mint_aq(id(2), addr(0xAA), 1000, 45, 50).unwrap_err();
        assert!(matches!(err, PoolError::BackwardsTime { .. }));
    }

    #[test]
    fn rate_cap_enforced_per_wallet() {
        // Cap = 3 per window of 60 epochs. After 3 mints in tight
        // succession, the 4th must be rejected.
        let mut pool = AttentionPool::new(3, 60).unwrap();
        for i in 1..=3u8 {
            pool.mint_aq(id(i), addr(0xAA), 1000, 45, i as u64).unwrap();
        }
        let err = pool.mint_aq(id(4), addr(0xAA), 1000, 45, 4).unwrap_err();
        assert!(matches!(err, PoolError::RateCapped { .. }));
    }

    #[test]
    fn rate_cap_resets_after_window_passes() {
        let mut pool = AttentionPool::new(2, 10).unwrap();
        pool.mint_aq(id(1), addr(0xAA), 1000, 45, 0).unwrap();
        pool.mint_aq(id(2), addr(0xAA), 1000, 45, 1).unwrap();
        // Within window: third mint blocked.
        assert!(pool.mint_aq(id(3), addr(0xAA), 1000, 45, 2).is_err());
        // Past window: cleared. At epoch 100, the window covers
        // [90, 100] and the earlier mints (epochs 0, 1) are outside.
        pool.mint_aq(id(4), addr(0xAA), 1000, 45, 100).unwrap();
        assert_eq!(pool.minted_in_window(&addr(0xAA), 100), 1);
    }

    #[test]
    fn rate_cap_is_per_wallet_not_global() {
        // Each wallet has its own cap; one wallet's flood doesn't
        // block another's mint.
        let mut pool = AttentionPool::new(2, 10).unwrap();
        pool.mint_aq(id(1), addr(0xAA), 1000, 45, 0).unwrap();
        pool.mint_aq(id(2), addr(0xAA), 1000, 45, 1).unwrap();
        // 0xAA is at cap. 0xBB should still be able to mint.
        pool.mint_aq(id(3), addr(0xBB), 1000, 45, 2).unwrap();
        assert_eq!(pool.minted_in_window(&addr(0xAA), 2), 2);
        assert_eq!(pool.minted_in_window(&addr(0xBB), 2), 1);
    }

    #[test]
    fn the_anti_sybil_guard_lives_as_a_test() {
        // Doctrine guard: "A single human's attention is bounded."
        // Even if a Sybil farms many AQs at full speed, the rate cap
        // hard-limits emission per (window, wallet). A wallet can't
        // emit more than `max_aq_per_window` per window, *no matter
        // how attentive* they pretend to be.
        let mut pool = AttentionPool::new(5, 60).unwrap();
        for i in 1..=5u8 {
            pool.mint_aq(id(i), addr(0xAA), 1000, 45, i as u64).unwrap();
        }
        // 6th mint at epoch 6 is rate-capped.
        let err = pool.mint_aq(id(6), addr(0xAA), 1000, 45, 6).unwrap_err();
        assert!(matches!(err, PoolError::RateCapped { .. }));
    }

    #[test]
    fn aqs_for_filters_by_holder() {
        let mut pool = AttentionPool::new(5, 60).unwrap();
        pool.mint_aq(id(1), addr(0xAA), 1000, 45, 0).unwrap();
        pool.mint_aq(id(2), addr(0xBB), 1000, 45, 1).unwrap();
        pool.mint_aq(id(3), addr(0xAA), 500, 45, 2).unwrap();
        assert_eq!(pool.aqs_for(&addr(0xAA)).len(), 2);
        assert_eq!(pool.aqs_for(&addr(0xBB)).len(), 1);
        assert_eq!(pool.aqs_for(&addr(0xCC)).len(), 0);
    }

    // Note: AttentionPool uses HashMap<AqId, AttentionQuantum>; JSON
    // can't serialize byte-array map keys directly. Bincode round-trips
    // work; a JSON wrapper would need to hex-encode the keys. Same
    // caveat as evaporchain-ssm's Residual / Strategy types.
}
