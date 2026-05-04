//! `PatinaToken` — the NFT itself.
//!
//! A `PatinaToken` is the on-chain record bundling:
//!
//! - identity (`token_id`, `minted_by`, `minted_at`)
//! - patina parameters (frozen at mint)
//! - current owner
//!
//! There is **no `pause()`, no `freeze()`, no `refresh()`**. Per
//! doctrine: "Owner cannot pause; only witness." The decay state is a
//! pure function of `(token_id, params, minted_at, epoch_now)`. The
//! only mutating operation is `transfer` — and even that doesn't
//! affect the patina curve.
//!
//! `witness(epoch_now)` is a read-only convenience: returns the
//! current `PatinaState` so callers (wallets, marketplaces) can render
//! the token without manually composing `patina_score` and
//! `derive_state`.

use evaporchain_types::{AccountAddress, Energy, Epoch, HalfLife};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::entropy::{derive_state, PatinaState};
use crate::patina::{patina_score, PatinaError, PatinaParams};

/// 32-byte opaque token handle. Caller picks (often a hash of mint
/// inputs); used as the deterministic seed for the off-chain shader's
/// procedural cracks/foxing layout.
pub type TokenId = [u8; 32];

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TokenError {
    #[error("patina parameter setup failed: {0}")]
    Patina(#[from] PatinaError),
    #[error("transfer caller {caller:?} is not the current owner {owner:?}")]
    NotOwner {
        caller: AccountAddress,
        owner: AccountAddress,
    },
    #[error("transfer to self is a no-op")]
    SelfTransfer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatinaToken {
    pub id: TokenId,
    pub minted_by: AccountAddress,
    pub minted_at_epoch: Epoch,
    pub params: PatinaParams,
    pub owner: AccountAddress,
}

impl PatinaToken {
    /// Mint a new token with custom patina params.
    pub fn mint(
        id: TokenId,
        minted_by: AccountAddress,
        params: PatinaParams,
        minted_at_epoch: Epoch,
    ) -> Self {
        Self {
            id,
            minted_by,
            minted_at_epoch,
            params,
            owner: minted_by,
        }
    }

    /// Mint with the doctrine-default 15% floor "ruined-beautiful"
    /// asymptote.
    pub fn mint_ruined_beautiful(
        id: TokenId,
        minted_by: AccountAddress,
        initial: Energy,
        half_life: HalfLife,
        minted_at_epoch: Epoch,
    ) -> Result<Self, TokenError> {
        let params = PatinaParams::ruined_beautiful(initial, half_life)?;
        Ok(Self::mint(id, minted_by, params, minted_at_epoch))
    }

    /// Transfer ownership to a new wallet. Caller must be the current
    /// owner. Self-transfers are rejected (no useful state change,
    /// keeps the audit trail clean).
    pub fn transfer(
        &mut self,
        caller: AccountAddress,
        new_owner: AccountAddress,
    ) -> Result<(), TokenError> {
        if caller != self.owner {
            return Err(TokenError::NotOwner {
                caller,
                owner: self.owner,
            });
        }
        if new_owner == self.owner {
            return Err(TokenError::SelfTransfer);
        }
        self.owner = new_owner;
        Ok(())
    }

    /// Read-only: numeric patina score at `epoch_now`.
    pub fn score_at(&self, epoch_now: Epoch) -> Energy {
        patina_score(&self.params, self.minted_at_epoch, epoch_now)
    }

    /// Read-only: full entropy state at `epoch_now`. Wallets and
    /// marketplaces should call this to render the token; the shader
    /// consumes (state, token_id) — `token_id` is the procedural
    /// seed so cracks etc land at canonical positions.
    pub fn witness(&self, epoch_now: Epoch) -> PatinaState {
        let score = self.score_at(epoch_now);
        derive_state(&self.params, score)
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

    #[test]
    fn mint_records_creator_as_initial_owner() {
        let t = PatinaToken::mint_ruined_beautiful(id(1), addr(0xAA), 1000, 100, 10).unwrap();
        assert_eq!(t.owner, addr(0xAA));
        assert_eq!(t.minted_by, addr(0xAA));
        assert_eq!(t.minted_at_epoch, 10);
        assert_eq!(t.params.floor, 150); // 15% of 1000
    }

    #[test]
    fn transfer_requires_current_owner() {
        let mut t =
            PatinaToken::mint_ruined_beautiful(id(1), addr(0xAA), 1000, 100, 10).unwrap();
        // Non-owner cannot transfer.
        let err = t.transfer(addr(0xBB), addr(0xCC)).unwrap_err();
        assert!(matches!(err, TokenError::NotOwner { .. }));
        // Owner can transfer.
        t.transfer(addr(0xAA), addr(0xCC)).unwrap();
        assert_eq!(t.owner, addr(0xCC));
    }

    #[test]
    fn self_transfer_rejected() {
        let mut t =
            PatinaToken::mint_ruined_beautiful(id(1), addr(0xAA), 1000, 100, 10).unwrap();
        let err = t.transfer(addr(0xAA), addr(0xAA)).unwrap_err();
        assert_eq!(err, TokenError::SelfTransfer);
    }

    #[test]
    fn transfer_does_not_alter_decay_curve() {
        // Doctrine: owner cannot pause; only witness. Transfer must NOT
        // refresh the patina or change the curve in any way.
        let mut t =
            PatinaToken::mint_ruined_beautiful(id(1), addr(0xAA), 1000, 100, 0).unwrap();
        let score_before = t.score_at(100);
        t.transfer(addr(0xAA), addr(0xBB)).unwrap();
        let score_after = t.score_at(100);
        assert_eq!(score_before, score_after);
        // minted_at_epoch unchanged.
        assert_eq!(t.minted_at_epoch, 0);
    }

    #[test]
    fn witness_at_mint_is_pristine() {
        let t = PatinaToken::mint_ruined_beautiful(id(1), addr(0xAA), 1000, 100, 50).unwrap();
        let st = t.witness(50);
        assert_eq!(st.cracks, 0);
        assert_eq!(st.desaturation, 0);
        assert_eq!(st.foxing, 0);
        assert_eq!(st.edge_fray, 0);
        assert_eq!(st.score, 1000);
    }

    #[test]
    fn witness_at_far_future_is_at_floor() {
        let t = PatinaToken::mint_ruined_beautiful(id(1), addr(0xAA), 1000, 10, 0).unwrap();
        // After 1000 epochs (100 half-lives), decayable portion ≈ 0.
        let st = t.witness(1000);
        assert_eq!(st.score, 150); // floor
        assert_eq!(st.cracks, 100);
        assert_eq!(st.desaturation, 100);
        assert_eq!(st.foxing, 100);
        assert_eq!(st.edge_fray, 100);
    }

    #[test]
    fn round_trip_serde() {
        let t = PatinaToken::mint_ruined_beautiful(id(7), addr(0xAB), 5000, 365, 42).unwrap();
        let s = serde_json::to_string(&t).unwrap();
        let back: PatinaToken = serde_json::from_str(&s).unwrap();
        assert_eq!(t, back);
    }

    #[test]
    fn no_pause_method_exists() {
        // Doctrine guard: this is a compile-time / API-shape claim,
        // enforced by NOT having any method that mutates params or
        // shifts minted_at_epoch. The test is here as a regression
        // marker — if a future contributor adds pause()/freeze(), this
        // test's documentation should make them think twice.
        let t = PatinaToken::mint_ruined_beautiful(id(1), addr(0xAA), 1000, 100, 0).unwrap();
        // Read-only methods only:
        let _ = t.score_at(50);
        let _ = t.witness(50);
        // No pause() or freeze() to call.
    }
}
