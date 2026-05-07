//! `HeirloomNft` — kin-graph heirloom NFT state machine.

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct TokenId(pub [u8; 32]);

#[derive(Debug, Error, PartialEq, Eq)]
pub enum HeirloomError {
    #[error("zero initial energy")]
    ZeroEnergy,
    #[error("token already escheated to commons")]
    Escheated,
    #[error("holder declared as their own heir — kin graph cannot be self-loop")]
    SelfLoopKin,
    #[error("non-monotone tick: incoming {incoming} ≤ last {last}")]
    NonMonotoneTick { incoming: u64, last: u64 },
    #[error("inherit() called but holder is alive — death cert required")]
    HolderStillAlive,
    #[error("heir set is empty and holder dead — token escheated to commons")]
    NoLiveHeirs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HeirState {
    Live,
    /// Heir certified dead; cannot inherit.
    Dead,
    /// Heir refused / explicitly opted out.
    Refused,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KinEdge {
    pub heir: [u8; 32],
    pub state: HeirState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeirloomNft {
    pub id: TokenId,
    pub holder: [u8; 32],
    /// True if the chain has certified the holder dead.
    pub holder_dead: bool,
    pub energy: u64,
    /// Ordered kin list — index 0 is highest priority.
    pub kin: Vec<KinEdge>,
    /// Number of inheritance hops already applied. Useful for
    /// generational provenance reads.
    pub generation: u32,
    /// True after escheat.
    pub escheated: bool,
    pub last_tick: u64,
}

impl HeirloomNft {
    pub fn mint(
        id: TokenId,
        founder: [u8; 32],
        initial_energy: u64,
        kin: Vec<KinEdge>,
        opened_at_tick: u64,
    ) -> Result<Self, HeirloomError> {
        if initial_energy == 0 {
            return Err(HeirloomError::ZeroEnergy);
        }
        for edge in &kin {
            if edge.heir == founder {
                return Err(HeirloomError::SelfLoopKin);
            }
        }
        Ok(Self {
            id,
            holder: founder,
            holder_dead: false,
            energy: initial_energy,
            kin,
            generation: 0,
            escheated: false,
            last_tick: opened_at_tick,
        })
    }

    /// Mark the holder dead. Inheritance is now allowed.
    pub fn certify_holder_death(&mut self) -> Result<(), HeirloomError> {
        if self.escheated {
            return Err(HeirloomError::Escheated);
        }
        self.holder_dead = true;
        Ok(())
    }

    /// Mark a heir (by 32-byte address) dead or refused.
    pub fn mark_heir_state(
        &mut self,
        heir: [u8; 32],
        state: HeirState,
    ) -> Result<(), HeirloomError> {
        if self.escheated {
            return Err(HeirloomError::Escheated);
        }
        if let Some(edge) = self.kin.iter_mut().find(|e| e.heir == heir) {
            edge.state = state;
        }
        Ok(())
    }

    /// Apply the chain's energy decay tick (separate from
    /// inheritance halving).
    pub fn tick_to(&mut self, now: u64, new_energy: u64) -> Result<(), HeirloomError> {
        if self.escheated {
            return Err(HeirloomError::Escheated);
        }
        if now <= self.last_tick {
            return Err(HeirloomError::NonMonotoneTick {
                incoming: now,
                last: self.last_tick,
            });
        }
        self.energy = new_energy;
        self.last_tick = now;
        Ok(())
    }

    /// Inherit: walk the kin list to find the highest-priority
    /// LIVE heir; transfer the token; halve the energy; bump
    /// generation. If no live heir, escheat.
    pub fn inherit(&mut self) -> Result<[u8; 32], HeirloomError> {
        if self.escheated {
            return Err(HeirloomError::Escheated);
        }
        if !self.holder_dead {
            return Err(HeirloomError::HolderStillAlive);
        }
        // Find the highest-priority live heir.
        let live_idx = self
            .kin
            .iter()
            .position(|e| matches!(e.state, HeirState::Live));
        match live_idx {
            None => {
                // No live heirs → escheat.
                self.escheated = true;
                Err(HeirloomError::NoLiveHeirs)
            }
            Some(idx) => {
                let new_holder = self.kin[idx].heir;
                // Apply inheritance-tax half-life: energy /= 2.
                self.energy /= 2;
                self.holder = new_holder;
                self.holder_dead = false;
                self.generation = self.generation.saturating_add(1);
                // The new holder's own kin graph would be set
                // afterwards via `set_kin`. Clear the previous
                // holder's kin to avoid stale priorities.
                self.kin.clear();
                Ok(new_holder)
            }
        }
    }

    /// Replace the kin list (used by the new holder after inheritance,
    /// or by any holder updating their will).
    pub fn set_kin(&mut self, kin: Vec<KinEdge>) -> Result<(), HeirloomError> {
        if self.escheated {
            return Err(HeirloomError::Escheated);
        }
        for edge in &kin {
            if edge.heir == self.holder {
                return Err(HeirloomError::SelfLoopKin);
            }
        }
        self.kin = kin;
        Ok(())
    }

    /// Reinforce: holder spends gas to refresh energy back up
    /// (caller passes the new energy after the chain's higher
    /// layer applies any cap). Heirs can use this AFTER inheriting
    /// to keep the heirloom from decaying further.
    pub fn reinforce(&mut self, new_energy: u64) -> Result<(), HeirloomError> {
        if self.escheated {
            return Err(HeirloomError::Escheated);
        }
        self.energy = new_energy;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tid(b: u8) -> TokenId {
        TokenId([b; 32])
    }
    fn alice() -> [u8; 32] {
        [0xAA; 32]
    }
    fn bob() -> [u8; 32] {
        [0xBB; 32]
    }
    fn carol() -> [u8; 32] {
        [0xCC; 32]
    }
    fn dan() -> [u8; 32] {
        [0xDD; 32]
    }

    fn live(addr: [u8; 32]) -> KinEdge {
        KinEdge {
            heir: addr,
            state: HeirState::Live,
        }
    }

    fn dead(addr: [u8; 32]) -> KinEdge {
        KinEdge {
            heir: addr,
            state: HeirState::Dead,
        }
    }

    fn refused(addr: [u8; 32]) -> KinEdge {
        KinEdge {
            heir: addr,
            state: HeirState::Refused,
        }
    }

    // ── construction ─────────────────────────────────────────────

    #[test]
    fn mint_initializes_at_generation_zero() {
        let n = HeirloomNft::mint(tid(1), alice(), 1000, vec![live(bob())], 0).unwrap();
        assert_eq!(n.generation, 0);
        assert_eq!(n.holder, alice());
        assert!(!n.holder_dead);
    }

    #[test]
    fn mint_rejects_zero_energy() {
        let err = HeirloomNft::mint(tid(1), alice(), 0, vec![live(bob())], 0).unwrap_err();
        assert_eq!(err, HeirloomError::ZeroEnergy);
    }

    #[test]
    fn mint_rejects_self_in_kin() {
        let err = HeirloomNft::mint(tid(1), alice(), 1000, vec![live(alice())], 0).unwrap_err();
        assert_eq!(err, HeirloomError::SelfLoopKin);
    }

    // ── inherit ──────────────────────────────────────────────────

    #[test]
    fn inherit_passes_to_highest_live_heir() {
        let mut n =
            HeirloomNft::mint(tid(1), alice(), 1000, vec![live(bob()), live(carol())], 0).unwrap();
        n.certify_holder_death().unwrap();
        let new_holder = n.inherit().unwrap();
        assert_eq!(new_holder, bob());
        assert_eq!(n.holder, bob());
        assert_eq!(n.energy, 500);
        assert_eq!(n.generation, 1);
    }

    #[test]
    fn inherit_skips_dead_heirs() {
        let mut n =
            HeirloomNft::mint(tid(1), alice(), 1000, vec![dead(bob()), live(carol())], 0).unwrap();
        n.certify_holder_death().unwrap();
        let new_holder = n.inherit().unwrap();
        assert_eq!(new_holder, carol());
    }

    #[test]
    fn inherit_skips_refused_heirs() {
        let mut n = HeirloomNft::mint(
            tid(1),
            alice(),
            1000,
            vec![refused(bob()), live(carol())],
            0,
        )
        .unwrap();
        n.certify_holder_death().unwrap();
        let new_holder = n.inherit().unwrap();
        assert_eq!(new_holder, carol());
    }

    #[test]
    fn inherit_with_no_live_heirs_escheats() {
        let mut n = HeirloomNft::mint(
            tid(1),
            alice(),
            1000,
            vec![dead(bob()), refused(carol())],
            0,
        )
        .unwrap();
        n.certify_holder_death().unwrap();
        let err = n.inherit().unwrap_err();
        assert_eq!(err, HeirloomError::NoLiveHeirs);
        assert!(n.escheated);
    }

    #[test]
    fn inherit_holder_alive_rejected() {
        let mut n = HeirloomNft::mint(tid(1), alice(), 1000, vec![live(bob())], 0).unwrap();
        let err = n.inherit().unwrap_err();
        assert_eq!(err, HeirloomError::HolderStillAlive);
    }

    #[test]
    fn inherit_clears_kin_for_new_holder() {
        let mut n =
            HeirloomNft::mint(tid(1), alice(), 1000, vec![live(bob()), live(carol())], 0).unwrap();
        n.certify_holder_death().unwrap();
        n.inherit().unwrap();
        assert!(n.kin.is_empty());
    }

    // ── set_kin ──────────────────────────────────────────────────

    #[test]
    fn set_kin_rejects_self() {
        let mut n = HeirloomNft::mint(tid(1), alice(), 1000, vec![live(bob())], 0).unwrap();
        let err = n.set_kin(vec![live(alice())]).unwrap_err();
        assert_eq!(err, HeirloomError::SelfLoopKin);
    }

    #[test]
    fn set_kin_replaces_full_list() {
        let mut n = HeirloomNft::mint(tid(1), alice(), 1000, vec![live(bob())], 0).unwrap();
        n.set_kin(vec![live(carol()), live(dan())]).unwrap();
        assert_eq!(n.kin.len(), 2);
        assert_eq!(n.kin[0].heir, carol());
    }

    // ── multi-generation halving ─────────────────────────────────

    #[test]
    fn multi_generation_halves_geometrically() {
        let mut n = HeirloomNft::mint(
            tid(1),
            alice(),
            1024, // 2^10
            vec![live(bob())],
            0,
        )
        .unwrap();
        // Gen 1: alice → bob, energy 512.
        n.certify_holder_death().unwrap();
        n.inherit().unwrap();
        n.set_kin(vec![live(carol())]).unwrap();
        assert_eq!(n.energy, 512);

        // Gen 2: bob → carol, energy 256.
        n.certify_holder_death().unwrap();
        n.inherit().unwrap();
        n.set_kin(vec![live(dan())]).unwrap();
        assert_eq!(n.energy, 256);

        // Gen 3: carol → dan, energy 128.
        n.certify_holder_death().unwrap();
        n.inherit().unwrap();
        assert_eq!(n.energy, 128);
        assert_eq!(n.generation, 3);
    }

    // ── reinforce ────────────────────────────────────────────────

    #[test]
    fn reinforce_restores_energy() {
        let mut n = HeirloomNft::mint(tid(1), alice(), 1000, vec![live(bob())], 0).unwrap();
        n.certify_holder_death().unwrap();
        n.inherit().unwrap();
        assert_eq!(n.energy, 500);
        n.reinforce(2000).unwrap();
        assert_eq!(n.energy, 2000);
    }

    // ── escheat is sticky ────────────────────────────────────────

    #[test]
    fn escheated_token_rejects_further_ops() {
        let mut n = HeirloomNft::mint(tid(1), alice(), 1000, vec![dead(bob())], 0).unwrap();
        n.certify_holder_death().unwrap();
        let _ = n.inherit(); // escheats
        assert!(n.escheated);
        // Subsequent ops fail.
        assert_eq!(
            n.certify_holder_death().unwrap_err(),
            HeirloomError::Escheated
        );
        assert!(matches!(
            n.tick_to(100, 1000).unwrap_err(),
            HeirloomError::Escheated
        ));
        assert_eq!(n.reinforce(500).unwrap_err(), HeirloomError::Escheated);
        assert_eq!(n.set_kin(vec![]).unwrap_err(), HeirloomError::Escheated);
    }

    // ── monotone tick ────────────────────────────────────────────

    #[test]
    fn non_monotone_tick_rejected() {
        let mut n = HeirloomNft::mint(tid(1), alice(), 1000, vec![live(bob())], 10).unwrap();
        let err = n.tick_to(5, 500).unwrap_err();
        assert!(matches!(err, HeirloomError::NonMonotoneTick { .. }));
    }

    // ── doctrine claim ────────────────────────────────────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "Singh-Heir is the first NFT primitive where
        // generational decay is structural. Each inheritance hop
        // halves the heirloom's energy. Across 4 generations of
        // unreinforced inheritance, an heirloom loses 15/16ths
        // of its value. Heirs who actively reinforce keep it
        // alive; heirs who don't watch it fade."

        let mut active = HeirloomNft::mint(tid(1), alice(), 16_000, vec![live(bob())], 0).unwrap();
        // Active line: bob inherits, immediately reinforces.
        active.certify_holder_death().unwrap();
        active.inherit().unwrap();
        active.reinforce(16_000).unwrap();
        active.set_kin(vec![live(carol())]).unwrap();
        active.certify_holder_death().unwrap();
        active.inherit().unwrap();
        active.reinforce(16_000).unwrap();
        // Active ends at 16_000 after 2 generations.
        assert_eq!(active.energy, 16_000);
        assert_eq!(active.generation, 2);

        let mut neglected =
            HeirloomNft::mint(tid(2), alice(), 16_000, vec![live(bob())], 0).unwrap();
        // Neglected line: nobody reinforces.
        neglected.certify_holder_death().unwrap();
        neglected.inherit().unwrap();
        neglected.set_kin(vec![live(carol())]).unwrap();
        neglected.certify_holder_death().unwrap();
        neglected.inherit().unwrap();
        neglected.set_kin(vec![live(dan())]).unwrap();
        neglected.certify_holder_death().unwrap();
        neglected.inherit().unwrap();
        // 16_000 → 8_000 → 4_000 → 2_000 over 3 hops.
        assert_eq!(neglected.energy, 2_000);
        assert_eq!(neglected.generation, 3);

        // Active line is 8x the energy of neglected.
        assert!(active.energy > neglected.energy * 5);
    }

    proptest::proptest! {
        #[test]
        fn property_n_inheritance_hops_halve_n_times(
            init_pow in 4u32..16u32,
            hops in 1u32..10u32,
        ) {
            // Starting energy = 2^init_pow; after `hops`
            // inheritance hops with no reinforcement, energy =
            // 2^(init_pow - hops) (saturating to 0 if hops > init_pow).
            let initial = 1u64 << init_pow;
            let mut n = HeirloomNft::mint(tid(1), alice(), initial, vec![live(bob())], 0).unwrap();
            let mut current_holder = alice();
            for h in 0..hops {
                let next: [u8; 32] = [0xF0 | (h as u8 & 0x0F); 32];
                if next == current_holder {
                    // skip pathological self-equality
                    return Ok(());
                }
                n.set_kin(vec![live(next)]).unwrap();
                n.certify_holder_death().unwrap();
                let _ = n.inherit();
                current_holder = next;
            }
            let expected = if hops as u32 > init_pow {
                0
            } else {
                1u64 << (init_pow - hops as u32)
            };
            proptest::prop_assert_eq!(n.energy, expected);
        }
    }
}
