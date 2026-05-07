//! `HalfLifeNft` — the NFT state machine with tier-aware decay.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use evaporchain_types::energy_at_epoch;

use crate::tier::TierLadder;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct TokenId(pub [u8; 32]);

#[derive(Debug, Error, PartialEq, Eq)]
pub enum NftError {
    #[error("non-monotone tick: incoming {incoming} ≤ last {last}")]
    NonMonotoneTick { incoming: u64, last: u64 },
    #[error("transfer to same holder is a no-op — would falsely reset retention")]
    TransferToSameHolder,
    #[error("zero initial energy")]
    ZeroInitialEnergy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HalfLifeNft {
    pub id: TokenId,
    pub holder: [u8; 32],
    /// Current decay-adjusted energy.
    pub energy: u64,
    /// Cumulative epochs the *current* holder has held this token.
    /// Reset to 0 on transfer.
    pub held_epochs_by_current_holder: u64,
    /// Last `tick_to` call's `now`.
    pub last_tick: u64,
    /// Tier ladder applied when computing the half-life.
    pub ladder: TierLadder,
}

impl HalfLifeNft {
    pub fn mint(
        id: TokenId,
        holder: [u8; 32],
        initial_energy: u64,
        opened_at_tick: u64,
        ladder: TierLadder,
    ) -> Result<Self, NftError> {
        if initial_energy == 0 {
            return Err(NftError::ZeroInitialEnergy);
        }
        Ok(Self {
            id,
            holder,
            energy: initial_energy,
            held_epochs_by_current_holder: 0,
            last_tick: opened_at_tick,
            ladder,
        })
    }

    /// Current tier index based on cumulative held-time.
    pub fn current_tier_index(&self) -> usize {
        self.ladder
            .tier_index_for(self.held_epochs_by_current_holder)
    }

    /// Advance the token's state to `now`. Energy decays each
    /// epoch using the tier-current half-life. The held-time
    /// counter increments by `now - last_tick`. **The tier may
    /// change mid-window**: we recompute it after each unit-epoch
    /// step (in batch via the closed-form per-tier exponential
    /// halving applied to the segment that this tier covers).
    pub fn tick_to(&mut self, now: u64) -> Result<(), NftError> {
        if now <= self.last_tick {
            return Err(NftError::NonMonotoneTick {
                incoming: now,
                last: self.last_tick,
            });
        }
        // Walk forward in segments where the tier is constant.
        // After each segment, check whether the next held_epochs
        // crossing promotes us to the next tier.
        let mut current = self.last_tick;
        let target = now;
        while current < target {
            let tier_idx = self.current_tier_index();
            let tier = self.ladder.tier_at(tier_idx);

            // How long until the next tier promotion (if any)?
            let next_threshold_held = self
                .ladder
                .rungs
                .get(tier_idx + 1)
                .map(|t| t.min_held_epochs)
                .unwrap_or(u64::MAX);
            let held_remaining_until_promotion =
                next_threshold_held.saturating_sub(self.held_epochs_by_current_holder);

            // Epochs we can advance in this tier.
            let advance = (target - current)
                .min(held_remaining_until_promotion)
                .max(1);

            // Decay energy by `advance` epochs of half-life H, via
            // the canonical Coq-verified `evaporchain_types::
            // energy_at_epoch`. DOCTRINE_PUNCH_LIST.md Layer 0 row 2
            // mandates that all decay is routed through this function;
            // the previous local `decay_energy` was the last
            // workspace-wide bypass and is now removed.
            self.energy = energy_at_epoch(self.energy, tier.half_life_epochs, advance);

            self.held_epochs_by_current_holder =
                self.held_epochs_by_current_holder.saturating_add(advance);
            current = current.saturating_add(advance);
        }
        self.last_tick = now;
        Ok(())
    }

    /// Transfer to a new holder. Resets retention clock + tier to 0.
    pub fn transfer(&mut self, new_holder: [u8; 32]) -> Result<(), NftError> {
        if new_holder == self.holder {
            return Err(NftError::TransferToSameHolder);
        }
        self.holder = new_holder;
        self.held_epochs_by_current_holder = 0;
        Ok(())
    }
}

// DOCTRINE_PUNCH_LIST.md Layer 0 row 2 — all decay routed through
// `evaporchain_types::energy_at_epoch`, the Coq-verified canonical
// function (theorem `energy_at_epoch_monotone` in
// `research/coq/EnergyDecayMonotonicity.v`). The local `decay_energy`
// was the last workspace bypass; removed 2026-05-06 to honour the
// Layer 0 invariant flagged by AUDIT_2026_05_06.md CRITICAL-3. The
// math was already identical to the canonical function (full halving
// via `>>` saturating at 64, plus linear within-halving
// interpolation rearranged from `after · (2h − r) / (2h)` to the
// equivalent `after − after · r / (2h)` form), so behaviour is
// preserved bit-exact across all existing test fixtures.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tier::default_ladder;

    fn tid(b: u8) -> TokenId {
        TokenId([b; 32])
    }
    fn alice() -> [u8; 32] {
        [0xAA; 32]
    }
    fn bob() -> [u8; 32] {
        [0xBB; 32]
    }

    fn fresh_token(initial_energy: u64) -> HalfLifeNft {
        HalfLifeNft::mint(tid(1), alice(), initial_energy, 0, default_ladder()).unwrap()
    }

    // ── construction ─────────────────────────────────────────────

    #[test]
    fn mint_initializes_at_tier_zero() {
        let n = fresh_token(1_000_000);
        assert_eq!(n.holder, alice());
        assert_eq!(n.energy, 1_000_000);
        assert_eq!(n.current_tier_index(), 0);
        assert_eq!(n.held_epochs_by_current_holder, 0);
    }

    #[test]
    fn mint_rejects_zero_energy() {
        let err = HalfLifeNft::mint(tid(1), alice(), 0, 0, default_ladder()).unwrap_err();
        assert_eq!(err, NftError::ZeroInitialEnergy);
    }

    // ── decay at tier 0 ──────────────────────────────────────────

    #[test]
    fn decay_at_tier_zero_halves_at_half_life() {
        let mut n = fresh_token(1_000_000);
        // Tier-0 half-life = 100. Tick to 100.
        n.tick_to(100).unwrap();
        // After exactly one half-life: ~50%.
        assert!(n.energy >= 500_000 && n.energy <= 510_000);
    }

    #[test]
    fn decay_compounds_across_multiple_half_lives() {
        let mut n = fresh_token(1_000_000);
        n.tick_to(300).unwrap();
        // 3 half-lives at tier 0 (since the held-time hits the
        // tier-1 threshold at 1000, well after 300).
        // 1.0 → 0.5 → 0.25 → 0.125 = 125_000.
        assert!(n.energy >= 124_000 && n.energy <= 126_000);
    }

    // ── tier promotion ───────────────────────────────────────────

    #[test]
    fn crossing_tier_threshold_promotes() {
        let mut n = fresh_token(1_000_000_000);
        // Tier-0 half-life = 100. After 1000 epochs at tier 0,
        // we'd have done 10 halvings: 10⁹ / 1024 ≈ 977k.
        // BUT the promotion happens AT epoch 1000, so we
        // get 10 halvings of tier-0 then transition to tier-1 at
        // h-l = 200.
        n.tick_to(1_000).unwrap();
        assert_eq!(n.current_tier_index(), 1);
    }

    #[test]
    fn tier_promotion_slows_subsequent_decay() {
        // Energy at end of tier-0 (after 1000 epochs). Then run
        // 200 more epochs at tier 1 — should be ONE tier-1 halving.
        let mut a = fresh_token(1_000_000_000);
        a.tick_to(1_000).unwrap();
        let energy_after_tier0 = a.energy;
        a.tick_to(1_200).unwrap();
        let energy_after_tier1_one_halving = a.energy;
        // Should be ≈ 50% of energy_after_tier0.
        assert!(
            energy_after_tier1_one_halving * 21 / 10 >= energy_after_tier0,
            "expected ~50%, got {energy_after_tier1_one_halving} from {energy_after_tier0}"
        );
        assert!(
            energy_after_tier1_one_halving * 19 / 10 <= energy_after_tier0,
            "expected ~50%, got {energy_after_tier1_one_halving} from {energy_after_tier0}"
        );
    }

    #[test]
    fn very_long_held_token_reaches_top_tier() {
        let mut n = fresh_token(u64::MAX);
        n.tick_to(60_000).unwrap();
        assert_eq!(n.current_tier_index(), 4);
    }

    // ── transfer resets retention ────────────────────────────────

    #[test]
    fn transfer_resets_retention_and_tier() {
        let mut n = fresh_token(1_000_000);
        n.tick_to(2_000).unwrap();
        assert_eq!(n.current_tier_index(), 1);
        n.transfer(bob()).unwrap();
        assert_eq!(n.holder, bob());
        assert_eq!(n.held_epochs_by_current_holder, 0);
        assert_eq!(n.current_tier_index(), 0);
    }

    #[test]
    fn transfer_to_same_holder_rejected() {
        let mut n = fresh_token(1_000_000);
        let err = n.transfer(alice()).unwrap_err();
        assert_eq!(err, NftError::TransferToSameHolder);
    }

    #[test]
    fn transfer_does_not_change_energy() {
        let mut n = fresh_token(1_000_000);
        n.tick_to(50).unwrap();
        let energy_before = n.energy;
        n.transfer(bob()).unwrap();
        assert_eq!(n.energy, energy_before);
    }

    // ── monotone tick ─────────────────────────────────────────────

    #[test]
    fn non_monotone_tick_rejected() {
        let mut n = fresh_token(1_000_000);
        n.tick_to(50).unwrap();
        let err = n.tick_to(50).unwrap_err();
        assert!(matches!(err, NftError::NonMonotoneTick { .. }));
        let err = n.tick_to(20).unwrap_err();
        assert!(matches!(err, NftError::NonMonotoneTick { .. }));
    }

    // ── decay-energy math (now via canonical energy_at_epoch) ────
    //
    // These tests previously called the local `decay_energy(energy,
    // epochs, h)` helper that was removed per AUDIT_2026_05_06.md
    // CRITICAL-3. They now exercise the canonical
    // `evaporchain_types::energy_at_epoch(initial, half_life,
    // epochs_elapsed)` directly. Argument order differs (swap
    // epochs ↔ half_life); fixed-point integer values are
    // bit-identical because the math is the same.

    #[test]
    fn decay_energy_zero_input_zero() {
        assert_eq!(energy_at_epoch(0, 50, 100), 0);
    }

    #[test]
    fn decay_energy_zero_epochs_unchanged() {
        assert_eq!(energy_at_epoch(1_000_000, 100, 0), 1_000_000);
    }

    #[test]
    fn decay_energy_one_half_life_halves() {
        let r = energy_at_epoch(1_000_000, 100, 100);
        assert_eq!(r, 500_000);
    }

    #[test]
    fn decay_energy_many_half_lives_decays_to_zero() {
        let r = energy_at_epoch(1_000_000, 100, 10_000);
        assert_eq!(r, 0);
    }

    #[test]
    fn decay_energy_partial_period_interpolates() {
        // At exactly half a half-life: factor = 1 - 50/(2·100) = 0.75
        let r = energy_at_epoch(1_000_000, 100, 50);
        assert_eq!(r, 750_000);
    }

    // ── doctrine claim ────────────────────────────────────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "Half-Life NFTs reward retention, not trading.
        // Two collectors. Loyal Lina holds 5_000 epochs and
        // climbs to tier 1 (half-life 200). Mercenary Mike
        // transfers every 500 epochs and stays at tier 0
        // (half-life 100). After the same total chain time,
        // Lina's NFT is far healthier than Mike's."
        //
        // Math: tier-0 half-life=100, tier-1 half-life=200.
        // - Lina tick_to(5_000): 0..1_000 = 10 tier-0 halvings,
        //   1_000..5_000 = 20 tier-1 halvings. Total 30 halvings.
        // - Mike: 10 cycles × 5 halvings each = 50 halvings.
        // With initial = 2^60: Lina = 2^30 ≈ 1.07B; Mike = 2^10 = 1024.
        // Ratio ≈ 1M ⇒ comfortably > 100x.
        let initial: u64 = 1 << 60;

        let lina_token = {
            let mut t = HalfLifeNft::mint(tid(1), alice(), initial, 0, default_ladder()).unwrap();
            t.tick_to(5_000).unwrap();
            t
        };

        let mike_token = {
            let mut t = HalfLifeNft::mint(tid(2), alice(), initial, 0, default_ladder()).unwrap();
            let mut now = 0u64;
            for cycle in 0..10 {
                now += 500;
                t.tick_to(now).unwrap();
                let next_holder = if cycle % 2 == 0 { bob() } else { alice() };
                t.transfer(next_holder).unwrap();
            }
            t
        };

        assert!(
            lina_token.energy > mike_token.energy * 100,
            "lina={} mike={} — loyal collector should be 100x healthier",
            lina_token.energy,
            mike_token.energy
        );
    }

    proptest::proptest! {
        #[test]
        fn property_decay_is_monotone_non_increasing(
            initial in 1u64..u64::MAX/2,
            t1 in 1u64..1000u64,
            t2 in 0u64..1000u64,
        ) {
            // Running tick_to twice with strictly increasing times
            // never increases energy.
            let mut n = fresh_token(initial);
            n.tick_to(t1).unwrap();
            let e1 = n.energy;
            let t2_actual = t1 + 1 + t2;
            n.tick_to(t2_actual).unwrap();
            let e2 = n.energy;
            proptest::prop_assert!(e2 <= e1);
        }

        #[test]
        fn property_transfer_does_not_affect_energy(
            initial in 1u64..u64::MAX/2,
            elapsed in 1u64..5_000u64,
        ) {
            let mut n = fresh_token(initial);
            n.tick_to(elapsed).unwrap();
            let e_before = n.energy;
            n.transfer(bob()).unwrap();
            proptest::prop_assert_eq!(n.energy, e_before);
        }
    }
}
