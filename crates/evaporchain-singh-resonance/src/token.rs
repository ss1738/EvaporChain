//! `ResonanceToken` — the on-chain Vital-Sign NFT.
//!
//! Composes `EngagementWindow` (decaying attention) + `CouplingParams`
//! (engagement → effective half-life) into a token whose decay rate
//! responds to whether anyone is currently witnessing it.
//!
//! Witnessing = `register_engagement(weight)`. Anyone can witness any
//! token; weight is chain-policy (1 for view, N for derivative mint,
//! etc. — left to the caller).
//!
//! Energy at any future epoch follows the standard `energy_at_epoch`
//! rule with the *currently effective* half-life. Because effective
//! half-life depends on attention which itself decays, energy
//! evaluation is a small composition:
//!
//! ```text
//!     attention_now = attention_at(epoch_now)
//!     effective_hl = effective_half_life(base_hl, attention_now)
//!     energy_now = energy_at_epoch(cached_energy, effective_hl,
//!                                  epoch_now - last_anchor)
//! ```
//!
//! V1 simplification: we re-anchor `cached_energy` on every
//! `register_engagement` call, so `energy_at_epoch` integrates over
//! the most-recent engagement window only. This keeps math integer-
//! only and validator-cheap. A V2 might integrate piecewise across
//! tier transitions, but for V1 the re-anchor is sound (it's a
//! conservative under-estimate of energy loved tokens accumulate).

use evaporchain_types::{energy_at_epoch, AccountAddress, Energy, Epoch, HalfLife};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::coupling::{effective_half_life, CouplingError, CouplingParams};
use crate::engagement::{EngagementWindow, EngagementWindowError};

pub type TokenId = [u8; 32];

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TokenError {
    #[error("initial_energy must be > 0")]
    ZeroInitial,
    #[error("base_half_life must be > 0")]
    ZeroBaseHalfLife,
    #[error("transfer caller {caller:?} is not the owner {owner:?}")]
    NotOwner {
        caller: AccountAddress,
        owner: AccountAddress,
    },
    #[error("self-transfer is a no-op")]
    SelfTransfer,
    #[error("token has fully evaporated; transfers blocked")]
    Evaporated,
    #[error("engagement: {0}")]
    Engagement(#[from] EngagementWindowError),
    #[error("coupling: {0}")]
    Coupling(#[from] CouplingError),
    #[error("engagement weight must be > 0")]
    ZeroEngagement,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResonanceToken {
    pub id: TokenId,
    pub minted_by: AccountAddress,
    pub owner: AccountAddress,
    /// Energy at the most recent re-anchor (mint or engagement).
    pub cached_energy: Energy,
    /// Epoch the cached_energy was last set.
    pub anchor_epoch: Epoch,
    pub base_half_life: HalfLife,
    pub coupling: CouplingParams,
    pub engagement: EngagementWindow,
}

impl ResonanceToken {
    /// Mint a Resonance token. Initial attention is zero; the token
    /// starts decaying at `min_scale × base_half_life` until someone
    /// witnesses it. (Loved-art-slows is conditional on attention,
    /// not granted.)
    pub fn mint(
        id: TokenId,
        minted_by: AccountAddress,
        initial_energy: Energy,
        base_half_life: HalfLife,
        coupling: CouplingParams,
        attention_half_life: HalfLife,
        minted_at_epoch: Epoch,
    ) -> Result<Self, TokenError> {
        if initial_energy == 0 {
            return Err(TokenError::ZeroInitial);
        }
        if base_half_life == 0 {
            return Err(TokenError::ZeroBaseHalfLife);
        }
        let engagement = EngagementWindow::new(attention_half_life, minted_at_epoch)?;
        Ok(Self {
            id,
            minted_by,
            owner: minted_by,
            cached_energy: initial_energy,
            anchor_epoch: minted_at_epoch,
            base_half_life,
            coupling,
            engagement,
        })
    }

    /// Effective half-life right now given the decayed attention.
    pub fn effective_half_life_at(&self, epoch_now: Epoch) -> Result<HalfLife, TokenError> {
        let attn = self.engagement.attention_at(epoch_now);
        Ok(effective_half_life(
            self.base_half_life,
            attn,
            &self.coupling,
        )?)
    }

    /// Current energy at `epoch_now`. V1 evaluation: uses the
    /// effective half-life *as of now* applied to the elapsed time
    /// since `anchor_epoch`. Conservative; loved tokens get re-anchor
    /// boosts whenever engagement registers.
    pub fn energy_at(&self, epoch_now: Epoch) -> Result<Energy, TokenError> {
        let elapsed = epoch_now.saturating_sub(self.anchor_epoch);
        let h_eff = self.effective_half_life_at(epoch_now)?;
        Ok(energy_at_epoch(self.cached_energy, h_eff, elapsed))
    }

    pub fn is_evaporated(&self, epoch_now: Epoch) -> bool {
        self.energy_at(epoch_now).map(|e| e == 0).unwrap_or(false)
    }

    /// Register engagement. Anyone can witness any token; the chain
    /// counts the weight. Re-anchors `cached_energy` to the current
    /// computed energy at `epoch_now`, then decays the engagement
    /// window to `epoch_now`, then adds the weight.
    pub fn register_engagement(
        &mut self,
        weight: Energy,
        epoch_now: Epoch,
    ) -> Result<(), TokenError> {
        if weight == 0 {
            return Err(TokenError::ZeroEngagement);
        }
        // Re-anchor energy to epoch_now so the new effective half-life
        // (after this engagement boost) applies forward, not retroactive.
        let energy_now = self.energy_at(epoch_now)?;
        self.cached_energy = energy_now;
        self.anchor_epoch = epoch_now;
        // Then bump the engagement window.
        self.engagement.register(epoch_now, weight)?;
        Ok(())
    }

    /// Transfer to a new owner. Requires caller==owner. Evaporated
    /// tokens refuse all transfers.
    pub fn transfer(
        &mut self,
        caller: AccountAddress,
        new_owner: AccountAddress,
        epoch_now: Epoch,
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
        if self.is_evaporated(epoch_now) {
            return Err(TokenError::Evaporated);
        }
        self.owner = new_owner;
        Ok(())
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

    fn fresh(creator: u8) -> ResonanceToken {
        let coupling = CouplingParams::default_attention_curve(1000).unwrap();
        ResonanceToken::mint(id(0xFF), addr(creator), 10_000, 100, coupling, 100, 0).unwrap()
    }

    #[test]
    fn mint_rejects_zero_initial() {
        let coupling = CouplingParams::default_attention_curve(1000).unwrap();
        let err = ResonanceToken::mint(id(1), addr(0xAA), 0, 100, coupling, 100, 0).unwrap_err();
        assert_eq!(err, TokenError::ZeroInitial);
    }

    #[test]
    fn mint_rejects_zero_base_half_life() {
        let coupling = CouplingParams::default_attention_curve(1000).unwrap();
        let err = ResonanceToken::mint(id(1), addr(0xAA), 1000, 0, coupling, 100, 0).unwrap_err();
        assert_eq!(err, TokenError::ZeroBaseHalfLife);
    }

    #[test]
    fn newly_minted_token_is_at_min_scale() {
        // Doctrine: tokens start at zero attention, so they decay at
        // the *minimum* effective half-life until someone witnesses.
        let t = fresh(0xAA);
        let h_eff = t.effective_half_life_at(0).unwrap();
        // min_scale = 50 bp = 0.5× base; base=100; so 50.
        assert_eq!(h_eff, 50);
    }

    #[test]
    fn ignored_token_dies_faster_than_engaged_token() {
        // Doctrine claim: loved art slows; ignored art accelerates.
        // Construct two identical tokens; engage one, ignore the other.
        let mut ignored = fresh(0xAA);
        let mut engaged = fresh(0xAA);
        // Engage the second token at every epoch with weight 200,
        // sustaining attention near saturation.
        for ep in 0..50u64 {
            engaged.register_engagement(200, ep).unwrap();
        }
        // After the same elapsed time, the engaged one has more energy.
        let after_ignored = ignored.energy_at(50).unwrap();
        let after_engaged = engaged.energy_at(50).unwrap();
        assert!(
            after_engaged > after_ignored,
            "engaged ({after_engaged}) must outlive ignored ({after_ignored})"
        );
    }

    #[test]
    fn engagement_at_saturation_brings_token_to_base_rate() {
        let mut t = fresh(0xAA);
        // Boost attention to saturation in one go.
        t.register_engagement(1000, 0).unwrap();
        let h_eff = t.effective_half_life_at(0).unwrap();
        // mid_scale = 100 bp = 1× base; base=100; so 100.
        assert_eq!(h_eff, 100);
    }

    #[test]
    fn massive_engagement_approaches_max_but_does_not_reach_it() {
        // Anti-Black-Mirror property: even the most engaged token
        // can't be pinned to immortality.
        let mut t = fresh(0xAA);
        t.register_engagement(1_000_000, 0).unwrap();
        let h_eff = t.effective_half_life_at(0).unwrap();
        let max_possible = 100 * 800 / 100; // 800
        assert!(
            h_eff < max_possible,
            "h_eff={h_eff} must be < max={max_possible}"
        );
        assert!(h_eff > 100, "should be much above mid");
    }

    #[test]
    fn yesterdays_likes_evaporate_on_the_token_too() {
        // Doctrine: a wave of attention followed by silence still
        // leads to death. Ramp up engagement, then go silent.
        let mut t = fresh(0xAA);
        // Big engagement burst at epoch 0.
        for _ in 0..100 {
            t.register_engagement(1000, 0).unwrap();
        }
        let h_now = t.effective_half_life_at(0).unwrap();
        // Far in the future, attention has decayed away — effective
        // half-life slid back toward the min.
        let h_later = t.effective_half_life_at(100_000).unwrap();
        assert!(
            h_later < h_now,
            "ignored later: {h_later} should be < {h_now}"
        );
    }

    #[test]
    fn register_engagement_zero_weight_rejected() {
        let mut t = fresh(0xAA);
        let err = t.register_engagement(0, 0).unwrap_err();
        assert_eq!(err, TokenError::ZeroEngagement);
    }

    #[test]
    fn transfer_after_evaporation_blocked() {
        // Tiny initial + small base half-life + min-scale (no
        // engagement) ⇒ token dies fast.
        let coupling = CouplingParams::default_attention_curve(1000).unwrap();
        let mut t = ResonanceToken::mint(id(1), addr(0xAA), 4, 1, coupling, 100, 0).unwrap();
        // After many half-lives at min_scale (0.5×base=0), token is dead.
        let err = t.transfer(addr(0xAA), addr(0xBB), 10_000).unwrap_err();
        assert_eq!(err, TokenError::Evaporated);
    }

    #[test]
    fn transfer_requires_owner() {
        let mut t = fresh(0xAA);
        let err = t.transfer(addr(0xBB), addr(0xCC), 10).unwrap_err();
        assert!(matches!(err, TokenError::NotOwner { .. }));
    }

    #[test]
    fn the_critique_lives_as_a_test() {
        // Doctrine framing: "exposes which art we are actually
        // sustaining with attention vs which we have abandoned."
        // Operationalised: mint two tokens; sustain one; abandon the
        // other; far in the future, the sustained one has higher
        // energy AND the abandoned one is closer to evaporation.
        let mut sustained = fresh(0xAA);
        let mut abandoned = fresh(0xBB);
        // Sustain attention on the first across 100 epochs.
        for ep in (0..1000).step_by(10) {
            sustained.register_engagement(500, ep).unwrap();
        }
        // Abandoned never sees engagement.
        let s_e = sustained.energy_at(1000).unwrap();
        let a_e = abandoned.energy_at(1000).unwrap();
        assert!(
            s_e > a_e,
            "sustained ({s_e}) should outlive abandoned ({a_e})"
        );
    }

    #[test]
    fn round_trip_serde() {
        let mut t = fresh(0xAA);
        t.register_engagement(50, 5).unwrap();
        let s = serde_json::to_string(&t).unwrap();
        let back: ResonanceToken = serde_json::from_str(&s).unwrap();
        assert_eq!(t, back);
    }

    /// T1.20 — transfer to self (caller == owner == new_owner) is
    /// rejected with SelfTransfer (line 170).
    #[test]
    fn t1_20_self_transfer_rejected() {
        let mut t = fresh(0xAA);
        let err = t.transfer(addr(0xAA), addr(0xAA), 10).unwrap_err();
        assert_eq!(err, TokenError::SelfTransfer);
    }
}
