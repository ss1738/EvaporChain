//! End-to-end integration tests for evaporchain-singh-heir.
//!
//! Non-trivial fixture: three-actor inheritance chain with dead/refused
//! heirs, chain-level energy decay ticks, and reinforce on the new holder.
//!
//!   Alice mints the heirloom:
//!     energy=8_192, tick=0
//!     kin=[Bob(Dead), Carol(Refused), Dan(Live)]
//!
//!   Tick 100: chain-level decay → energy=7_000.
//!
//!   Epoch A — Alice dies:
//!     certify_holder_death → inherit
//!     Skip Bob (Dead) → skip Carol (Refused) → take Dan (Live)
//!     Inheritance-tax: 7_000 → 3_500.  Generation = 1.  Kin list cleared.
//!
//!   Dan sets kin=[Eve(Live)].
//!   Tick 200: chain decay → energy=3_000.
//!
//!   Epoch B — Dan dies:
//!     certify_holder_death → inherit → Eve.
//!     3_000 → 1_500. Generation = 2.
//!
//!   Eve reinforces: 1_500 → 5_000.
//!
//! Doctrine claim (INVENTION_STACK.md §A5.3 / Singh-Heir):
//! "Singh-Heir is the first NFT primitive where generational decay is
//! structural. Each inheritance hop halves the heirloom's energy.
//! Heirs who actively reinforce keep it alive; heirs who don't watch
//! it fade. All-dead/refused kin → token escheats to the commons."
//!
//! Adversarial fixture: HolderStillAlive, NoLiveHeirs+Escheated,
//! SelfLoopKin, ZeroEnergy, NonMonotoneTick, Escheated-blocks-all,
//! unknown-heir mark is no-op.
//!
//! INVENTION_STACK §A5.3: Singh-Heir (Patrilithic Tokens).

use evaporchain_singh_heir::{
    token::{HeirState, KinEdge},
    HeirloomError, HeirloomNft, TokenId,
};

// ── Helpers ───────────────────────────────────────────────────────────────

fn tid(b: u8) -> TokenId {
    TokenId([b; 32])
}
fn alice() -> [u8; 32] { [0xAA; 32] }
fn bob()   -> [u8; 32] { [0xBB; 32] }
fn carol() -> [u8; 32] { [0xCC; 32] }
fn dan()   -> [u8; 32] { [0xDD; 32] }
fn eve()   -> [u8; 32] { [0xEE; 32] }
fn frank() -> [u8; 32] { [0xFF; 32] }

fn live(addr: [u8; 32]) -> KinEdge {
    KinEdge { heir: addr, state: HeirState::Live }
}
fn dead_k(addr: [u8; 32]) -> KinEdge {
    KinEdge { heir: addr, state: HeirState::Dead }
}
fn refused(addr: [u8; 32]) -> KinEdge {
    KinEdge { heir: addr, state: HeirState::Refused }
}

// ── Non-trivial fixture ───────────────────────────────────────────────────

#[test]
fn fixture_generation0_skips_dead_and_refused_heirs() {
    // Alice mints with Bob dead and Carol refused from the start.
    let mut n = HeirloomNft::mint(
        tid(1),
        alice(),
        8_192,
        vec![dead_k(bob()), refused(carol()), live(dan())],
        0,
    )
    .unwrap();
    assert_eq!(n.generation, 0);
    assert_eq!(n.holder, alice());

    // Chain-level decay tick.
    n.tick_to(100, 7_000).unwrap();
    assert_eq!(n.energy, 7_000);

    // Alice dies.
    n.certify_holder_death().unwrap();
    let next = n.inherit().unwrap();

    assert_eq!(next, dan(), "must skip Dead/Refused and land on Dan");
    assert_eq!(n.holder, dan());
    assert_eq!(n.energy, 3_500, "inheritance-tax halves 7_000 → 3_500");
    assert_eq!(n.generation, 1);
    assert!(!n.holder_dead, "new holder starts alive");
    assert!(n.kin.is_empty(), "kin list cleared after inherit");
}

#[test]
fn fixture_generation1_chain_decay_then_inherit_halves_decayed_value() {
    // Dan inherits at 4_000 from Alice (8_000 halved), then chain decay
    // runs before his death bringing energy to 3_000.
    let mut n = HeirloomNft::mint(tid(2), alice(), 8_000, vec![live(dan())], 0).unwrap();
    n.certify_holder_death().unwrap();
    n.inherit().unwrap();            // Gen 1: 8_000 → 4_000.
    n.set_kin(vec![live(eve())]).unwrap();
    assert_eq!(n.energy, 4_000);

    // Chain decay brings energy to 3_000 before Dan dies.
    n.tick_to(200, 3_000).unwrap();
    assert_eq!(n.energy, 3_000);

    // Dan dies; Eve inherits the chain-decayed value, not the original.
    n.certify_holder_death().unwrap();
    let next = n.inherit().unwrap();

    assert_eq!(next, eve());
    assert_eq!(n.energy, 1_500, "inheritance-tax halves chain-decayed 3_000 → 1_500");
    assert_eq!(n.generation, 2);
}

#[test]
fn fixture_reinforce_after_inherit_keeps_heirloom_alive() {
    let mut n = HeirloomNft::mint(tid(3), alice(), 1_024, vec![live(bob())], 0).unwrap();
    n.certify_holder_death().unwrap();
    n.inherit().unwrap();         // 1_024 → 512
    assert_eq!(n.energy, 512);

    n.reinforce(10_000).unwrap(); // Bob restores
    assert_eq!(n.energy, 10_000);
    assert_eq!(n.generation, 1);
    assert_eq!(n.holder, bob());
}

#[test]
fn fixture_three_generation_chain_halves_geometrically() {
    // 2^10 = 1024 → 512 → 256 → 128 across three hops.
    let mut n = HeirloomNft::mint(tid(4), alice(), 1_024, vec![live(bob())], 0).unwrap();

    // Gen 1: alice → bob.
    n.certify_holder_death().unwrap();
    n.inherit().unwrap();
    n.set_kin(vec![live(carol())]).unwrap();
    assert_eq!(n.energy, 512);
    assert_eq!(n.generation, 1);

    // Gen 2: bob → carol.
    n.certify_holder_death().unwrap();
    n.inherit().unwrap();
    n.set_kin(vec![live(dan())]).unwrap();
    assert_eq!(n.energy, 256);
    assert_eq!(n.generation, 2);

    // Gen 3: carol → dan.
    n.certify_holder_death().unwrap();
    n.inherit().unwrap();
    assert_eq!(n.energy, 128);
    assert_eq!(n.generation, 3);
}

#[test]
fn doctrine_active_vs_neglected_line_after_four_hops() {
    // Active: each heir reinforces back to 16_000 immediately.
    // Neglected: nobody reinforces. 16_000 → 8_000 → 4_000 → 2_000 → 1_000.
    let heirs: [[u8; 32]; 4] = [[0xA1; 32], [0xA2; 32], [0xA3; 32], [0xA4; 32]];

    let mut active = HeirloomNft::mint(tid(5), alice(), 16_000, vec![live(heirs[0])], 0).unwrap();
    for i in 0..4usize {
        active.certify_holder_death().unwrap();
        active.inherit().unwrap();
        active.reinforce(16_000).unwrap();
        if i < 3 {
            active.set_kin(vec![KinEdge { heir: heirs[i + 1], state: HeirState::Live }]).unwrap();
        }
    }
    assert_eq!(active.energy, 16_000, "active line stays at 16_000");
    assert_eq!(active.generation, 4);

    let mut neglected =
        HeirloomNft::mint(tid(6), alice(), 16_000, vec![live(heirs[0])], 0).unwrap();
    for i in 0..4usize {
        neglected.certify_holder_death().unwrap();
        neglected.inherit().unwrap();
        if i < 3 {
            neglected
                .set_kin(vec![KinEdge { heir: heirs[i + 1], state: HeirState::Live }])
                .unwrap();
        }
    }
    assert_eq!(neglected.energy, 1_000, "neglected: 16_000→8_000→4_000→2_000→1_000");
    assert!(active.energy > neglected.energy * 10, "active line is 16× the neglected line");
}

#[test]
fn doctrine_kin_priority_ordering_is_deterministic() {
    // First live heir wins. Dead/refused heirs earlier in the list are skipped.
    let kin = vec![dead_k(bob()), refused(carol()), live(dan()), live(eve())];
    let mut n = HeirloomNft::mint(tid(7), alice(), 1_000, kin, 0).unwrap();
    n.certify_holder_death().unwrap();
    let next = n.inherit().unwrap();
    assert_eq!(next, dan(), "first live heir (Dan, index 2) wins over Eve (index 3)");
}

#[test]
fn doctrine_mark_heir_state_changes_priority_outcome() {
    // Dan starts Live but is marked Refused just before inheritance → Eve wins.
    let mut n = HeirloomNft::mint(tid(8), alice(), 1_000, vec![live(dan()), live(eve())], 0)
        .unwrap();
    n.mark_heir_state(dan(), HeirState::Refused).unwrap();
    n.certify_holder_death().unwrap();
    let next = n.inherit().unwrap();
    assert_eq!(next, eve(), "Dan refused → Eve inherits");
}

// ── Adversarial fixture ───────────────────────────────────────────────────

#[test]
fn adversarial_inherit_without_death_cert_rejected() {
    let mut n = HeirloomNft::mint(tid(9), alice(), 1_000, vec![live(bob())], 0).unwrap();
    assert_eq!(n.inherit().unwrap_err(), HeirloomError::HolderStillAlive);
}

#[test]
fn adversarial_all_dead_heirs_escheats() {
    let mut n = HeirloomNft::mint(
        tid(10),
        alice(),
        1_000,
        vec![dead_k(bob()), refused(carol())],
        0,
    )
    .unwrap();
    n.certify_holder_death().unwrap();
    let err = n.inherit().unwrap_err();
    assert_eq!(err, HeirloomError::NoLiveHeirs);
    assert!(n.escheated, "token must be escheated after NoLiveHeirs");
}

#[test]
fn adversarial_zero_energy_mint_rejected() {
    assert_eq!(
        HeirloomNft::mint(tid(11), alice(), 0, vec![live(bob())], 0).unwrap_err(),
        HeirloomError::ZeroEnergy
    );
}

#[test]
fn adversarial_self_loop_kin_rejected_at_mint() {
    assert_eq!(
        HeirloomNft::mint(tid(12), alice(), 1_000, vec![live(alice())], 0).unwrap_err(),
        HeirloomError::SelfLoopKin
    );
}

#[test]
fn adversarial_self_loop_kin_rejected_in_set_kin() {
    let mut n = HeirloomNft::mint(tid(13), alice(), 1_000, vec![live(bob())], 0).unwrap();
    assert_eq!(n.set_kin(vec![live(alice())]).unwrap_err(), HeirloomError::SelfLoopKin);
}

#[test]
fn adversarial_non_monotone_tick_rejected() {
    let mut n = HeirloomNft::mint(tid(14), alice(), 1_000, vec![live(bob())], 100).unwrap();
    n.tick_to(200, 900).unwrap();
    let err = n.tick_to(150, 800).unwrap_err();
    assert!(
        matches!(err, HeirloomError::NonMonotoneTick { incoming: 150, last: 200 }),
        "backward tick must be rejected"
    );
    // Equal tick is also rejected.
    assert!(matches!(
        n.tick_to(200, 700).unwrap_err(),
        HeirloomError::NonMonotoneTick { .. }
    ));
}

#[test]
fn adversarial_escheated_token_blocks_all_mutations() {
    let mut n = HeirloomNft::mint(tid(15), alice(), 1_000, vec![dead_k(bob())], 0).unwrap();
    n.certify_holder_death().unwrap();
    let _ = n.inherit(); // escheats (no live heirs)
    assert!(n.escheated);

    assert_eq!(n.certify_holder_death().unwrap_err(),       HeirloomError::Escheated);
    assert_eq!(n.mark_heir_state(bob(), HeirState::Live).unwrap_err(), HeirloomError::Escheated);
    assert_eq!(n.tick_to(500, 1_000).unwrap_err(),          HeirloomError::Escheated);
    assert_eq!(n.reinforce(5_000).unwrap_err(),              HeirloomError::Escheated);
    assert_eq!(n.set_kin(vec![]).unwrap_err(),               HeirloomError::Escheated);
    assert_eq!(n.inherit().unwrap_err(),                     HeirloomError::Escheated);
}

#[test]
fn adversarial_mark_unknown_heir_is_noop() {
    let mut n = HeirloomNft::mint(tid(16), alice(), 1_000, vec![live(bob())], 0).unwrap();
    // frank() is not in the kin list — mark_heir_state silently does nothing.
    n.mark_heir_state(frank(), HeirState::Dead).unwrap();
    assert_eq!(n.kin[0].state, HeirState::Live, "Bob's Live state unchanged");
}

#[test]
fn adversarial_empty_kin_with_dead_holder_escheats() {
    let mut n = HeirloomNft::mint(tid(17), alice(), 1_000, vec![], 0).unwrap();
    n.certify_holder_death().unwrap();
    assert_eq!(n.inherit().unwrap_err(), HeirloomError::NoLiveHeirs);
    assert!(n.escheated);
}

// ── Cross-cuts ───────────────────────────────────────────────────────────

#[test]
fn serde_round_trip_preserves_all_fields() {
    let mut n = HeirloomNft::mint(
        tid(18),
        alice(),
        4_096,
        vec![live(bob()), dead_k(carol())],
        42,
    )
    .unwrap();
    n.tick_to(100, 3_000).unwrap();
    let json = serde_json::to_string(&n).unwrap();
    let back: HeirloomNft = serde_json::from_str(&json).unwrap();
    assert_eq!(n, back);
}

#[test]
fn inheritance_result_is_deterministic() {
    let kin = vec![dead_k(bob()), refused(carol()), live(dan())];
    let mut a = HeirloomNft::mint(tid(19), alice(), 2_000, kin.clone(), 0).unwrap();
    let mut b = HeirloomNft::mint(tid(19), alice(), 2_000, kin, 0).unwrap();
    a.certify_holder_death().unwrap();
    b.certify_holder_death().unwrap();
    assert_eq!(a.inherit().unwrap(), b.inherit().unwrap());
    assert_eq!(a.energy, b.energy);
}
