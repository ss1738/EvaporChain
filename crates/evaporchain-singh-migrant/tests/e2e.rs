//! End-to-end integration tests for evaporchain-singh-migrant.
//!
//! Non-trivial fixture: 3-hop "kula ring" vs stillness.
//! Params: initial=1_000, base_half_life=20, resting_threshold=30.
//!
//!   Kula ring (3 novel hops):
//!     Epoch  0: addr[0] mints. energy=1_000. rested=0.
//!     Epoch 10: addr[0]→addr[1] (novel).
//!       rest=10 (tier 1, h_eff=20). energy_at(10)=750.
//!       refund: 750+187=937. post=937. rested=10.
//!     Epoch 20: addr[1]→addr[2] (novel).
//!       rest=10 (tier 1). energy_at(20)=703.
//!       refund: 703+175=878. post=878. rested=20.
//!     Epoch 30: addr[2]→addr[3] (novel).
//!       rest=10 (tier 1). energy_at(30)=659.
//!       refund: 659+164=823. post=823. rested=30.
//!
//!   Still holder (same params, no transfer until epoch 30):
//!     rest=30. 30≥30 and 30<60: tier 2. h_eff=10.
//!     energy_at_epoch(1_000, 10, 30) = 1_000>>3 = 125.
//!
//!   Kula ring: 823 >> Still: 125. Ratio ~6.6×.
//!
//! Tier-2 exact arithmetic (initial=1_000, h=100, threshold=30):
//!   Alice holds for 50 epochs. rest=50, tier 2, h_eff=50.
//!   energy_at_epoch(1_000, 50, 50) = 1_000>>1 = 500.
//!   refund_amount(1_000, 500): 500+125=625. Novel → post=625.
//!
//! Revisit classification (same params, epoch 5 novel, epoch 20 revisit):
//!   Alice→Bob (novel, epoch 5): energy=975, refund→capped 1_000.
//!   Bob→Alice (revisit, epoch 20): Revisit outcome, rested_at stays at 5.
//!
//! Tier-3 acceleration (rest=70, threshold=30, h=100):
//!   tier3: h_eff=25. energy_at_epoch(1_000, 25, 70)=150.
//!   tier1: h_eff=100. energy_at_epoch(1_000, 100, 70)=650.
//!   Quadruple-speed decay confirmed.
//!
//! Arithmetic cross-checks:
//!   energy_at_epoch(1_000, 20, 10): full=0, rem=10, frac=1_000*10/40=250 → 750
//!   energy_at_epoch(937, 20, 10):   full=0, rem=10, frac=937*10/40=234  → 703
//!   energy_at_epoch(878, 20, 10):   full=0, rem=10, frac=878*10/40=219  → 659
//!   refund(1_000, 750)=750+187=937; refund(1_000, 703)=703+175=878; refund(1_000, 659)=659+164=823
//!   energy_at_epoch(1_000, 10, 30): full=3, rem=0, after=125 → 125
//!   energy_at_epoch(1_000, 50, 50): full=1, rem=0, after=500 → 500
//!   energy_at_epoch(1_000, 25, 70): full=2, rem=20, after=250, frac=250*20/50=100 → 150
//!   energy_at_epoch(1_000, 100, 70): full=0, rem=70, frac=1_000*70/200=350 → 650
//!
//! Doctrine claim (INVENTION_STACK.md §A5.3 / Singh-Migrant):
//! "The NFT that dies if you keep it. Resting past the threshold doubles
//!  λ; past 60 days it quadruples. Transfer to a novel wallet refunds 25%
//!  of current energy and resets the rest counter. Revisiting an old wallet
//!  is legal but gives no refund. Near-dead tokens get tiny refunds —
//!  the farm-and-relay attack doesn't restore much."
//!
//! Adversarial fixture: ZeroInitial, ZeroHalfLife, ZeroThreshold,
//! NotOwner, SelfTransfer, Evaporated.
//!
//! INVENTION_STACK §A5.3: Singh-Migrant (Wanderwrits).

use evaporchain_singh_migrant::{
    current_energy, effective_half_life, refund_amount, DecayError, MigrantToken, TokenError,
    TokenId, TransferOutcome,
};

// ── Helpers ───────────────────────────────────────────────────────────────

fn tid(b: u8) -> TokenId {
    [b; 32]
}
fn addr(b: u8) -> [u8; 32] {
    [b; 32]
}

/// Kula-ring params: initial=1_000, h=20, threshold=30, minted at 0.
fn kula_mint(owner: u8) -> MigrantToken {
    MigrantToken::mint(tid(owner), addr(owner), 1_000, 20, 30, 0).unwrap()
}

/// Tier-1/2 demo params: initial=1_000, h=100, threshold=30, minted at 0.
fn demo_mint(owner: u8) -> MigrantToken {
    MigrantToken::mint(tid(owner), addr(owner), 1_000, 100, 30, 0).unwrap()
}

// ── Non-trivial fixture ───────────────────────────────────────────────────

#[test]
fn fixture_kula_ring_3_hops_vs_stillness() {
    // Kula ring: three novel wallets at 10-epoch intervals.
    let mut ring = kula_mint(0xA0);

    // Hop 1 (epoch 10): addr[A0]→addr[A1]. Tier 1. energy=750. post=937.
    let out = ring.transfer(addr(0xA0), addr(0xA1), 10).unwrap();
    assert!(matches!(out, TransferOutcome::Novel { post_energy: 937 }));
    assert_eq!(ring.rested_at_epoch, 10);

    // Hop 2 (epoch 20): addr[A1]→addr[A2]. Tier 1. energy=703. post=878.
    let out = ring.transfer(addr(0xA1), addr(0xA2), 20).unwrap();
    assert!(matches!(out, TransferOutcome::Novel { post_energy: 878 }));
    assert_eq!(ring.rested_at_epoch, 20);

    // Hop 3 (epoch 30): addr[A2]→addr[A3]. Tier 1. energy=659. post=823.
    let out = ring.transfer(addr(0xA2), addr(0xA3), 30).unwrap();
    assert!(matches!(out, TransferOutcome::Novel { post_energy: 823 }));
    assert_eq!(ring.cached_energy, 823);
    assert_eq!(ring.rested_at_epoch, 30);
    assert_eq!(ring.visited_wallets.len(), 4);

    // Still holder (same params, no transfer): energy at epoch 30.
    let still_energy = kula_mint(0xB0).energy_at(30).unwrap();
    assert_eq!(
        still_energy, 125,
        "tier-2 at rest=30: energy_at_epoch(1_000,10,30)=125"
    );

    // Kula ring preserves ~6.6× more energy.
    assert!(
        ring.cached_energy > still_energy * 5,
        "kula ring ({}) must vastly outperform stillness ({})",
        ring.cached_energy,
        still_energy
    );
}

#[test]
fn fixture_novel_at_tier2_exact_arithmetic() {
    // Alice holds for 50 epochs (tier 2). exact energy=500. post=625.
    let mut t = demo_mint(0xAA);
    let out = t.transfer(addr(0xAA), addr(0xBB), 50).unwrap();
    assert!(
        matches!(out, TransferOutcome::Novel { post_energy: 625 }),
        "expected Novel{{625}}, got {out:?}"
    );
    assert_eq!(t.rested_at_epoch, 50, "rest counter resets to epoch 50");
    assert!(t.visited_wallets.contains(&addr(0xBB)));
}

#[test]
fn fixture_revisit_no_refund_no_rest_reset() {
    // Alice→Bob (novel, epoch 5) → Bob→Alice (revisit, epoch 20).
    let mut t = demo_mint(0xAA);

    // Novel at epoch 5: energy=975, refund→capped 1_000.
    t.transfer(addr(0xAA), addr(0xBB), 5).unwrap();
    assert_eq!(t.rested_at_epoch, 5);

    // Revisit at epoch 20: Bob sends back to Alice (already visited).
    // rest=15, tier1, energy_at_epoch(1_000, 100, 15)=925.
    let out = t.transfer(addr(0xBB), addr(0xAA), 20).unwrap();
    assert!(
        matches!(out, TransferOutcome::Revisit { post_energy: 925 }),
        "expected Revisit{{925}}, got {out:?}"
    );
    assert_eq!(
        t.rested_at_epoch, 5,
        "rest counter must NOT reset on revisit"
    );
    assert_eq!(
        t.visited_wallets.len(),
        2,
        "visited set must NOT grow on revisit"
    );
    assert_eq!(t.owner, addr(0xAA));
}

#[test]
fn fixture_tier3_quadruples_decay_rate() {
    // At rest=70 with threshold=30:
    //   tier3: h_eff=100/4=25. energy_at_epoch(1_000, 25, 70)=150.
    //   tier1: h_eff=100.       energy_at_epoch(1_000, 100, 70)=650.
    // Tier-3 is ~4.3× faster than tier-1.
    let tier3 = current_energy(1_000, 100, 0, 30, 70).unwrap();
    let tier1 = current_energy(1_000, 100, 0, 100, 70).unwrap(); // threshold=100 so tier1

    assert_eq!(tier3, 150, "tier3 at rest=70: 1_000>>2 - fractional = 150");
    assert_eq!(tier1, 650, "tier1 at rest=70: 1_000 - 350 = 650");
    assert!(tier1 > tier3 * 4, "tier3 must decay 4× faster than tier1");
}

#[test]
fn fixture_tier2_effective_half_life_is_halved() {
    // Exact: h_eff at rest=30 (threshold=30) must be base/2.
    assert_eq!(effective_half_life(100, 30, 30).unwrap(), 50);
    assert_eq!(effective_half_life(100, 59, 30).unwrap(), 50);
    // Tier 3: rest ≥ 60.
    assert_eq!(effective_half_life(100, 60, 30).unwrap(), 25);
    assert_eq!(effective_half_life(100, 200, 30).unwrap(), 25);
    // Tier 1: rest < threshold.
    assert_eq!(effective_half_life(100, 0, 30).unwrap(), 100);
    assert_eq!(effective_half_life(100, 29, 30).unwrap(), 100);
}

// ── Doctrine tests ────────────────────────────────────────────────────────

#[test]
fn doctrine_circulating_novel_wallets_beats_stillness() {
    // Two tokens. Active: novel hop at epoch 20. Passive: never moves.
    // At epoch 30, compare energies.
    let mut active = demo_mint(0xC0);
    active.transfer(addr(0xC0), addr(0xC1), 20).unwrap(); // Novel at tier-1 boundary.
    let active_energy = active.energy_at(30).unwrap();

    let passive = demo_mint(0xD0);
    let passive_energy = passive.energy_at(30).unwrap(); // Sitting into tier 2.

    assert!(
        active_energy > passive_energy,
        "active circulator ({active_energy}) must beat passive holder ({passive_energy})"
    );
}

#[test]
fn doctrine_farm_relay_attack_gets_diminishing_refunds() {
    // Near-dead token (current≈10) gets tiny refund; fresh gets full.
    let post_fresh = refund_amount(1_000, 1_000).unwrap();
    let post_dead = refund_amount(1_000, 10).unwrap();

    assert_eq!(post_fresh, 1_000, "fresh token capped at initial");
    assert_eq!(post_dead, 12, "near-dead: 10 + 25% = 12");
    assert!(
        post_fresh > post_dead * 50,
        "farm-relay attack yields 80× less refund"
    );
}

#[test]
fn doctrine_refund_fraction_is_twenty_five_percent() {
    // Doctrine default: 25% of current energy.
    let post = refund_amount(10_000, 400).unwrap();
    assert_eq!(post, 500, "400 + 25% = 500");
    // Verify REFUND_FRACTION_PCT constant is 25.
    assert_eq!(evaporchain_singh_migrant::REFUND_FRACTION_PCT, 25);
}

#[test]
fn doctrine_novel_ring_grows_visited_set_correctly() {
    let mut t = demo_mint(0xE0);
    assert_eq!(t.visited_wallets.len(), 1, "mint inserts founder");
    t.transfer(addr(0xE0), addr(0xE1), 5).unwrap();
    assert_eq!(t.visited_wallets.len(), 2);
    t.transfer(addr(0xE1), addr(0xE2), 10).unwrap();
    assert_eq!(t.visited_wallets.len(), 3);
    // Revisit: addr[E0] is already in visited — length stays at 3.
    t.transfer(addr(0xE2), addr(0xE0), 15).unwrap();
    assert_eq!(
        t.visited_wallets.len(),
        3,
        "revisit must not grow visited set"
    );
}

// ── Adversarial fixture ───────────────────────────────────────────────────

#[test]
fn adversarial_zero_initial_rejected() {
    assert_eq!(
        MigrantToken::mint(tid(1), addr(0xAA), 0, 100, 30, 0).unwrap_err(),
        TokenError::ZeroInitial
    );
}

#[test]
fn adversarial_zero_half_life_rejected() {
    assert_eq!(
        MigrantToken::mint(tid(1), addr(0xAA), 1_000, 0, 30, 0).unwrap_err(),
        TokenError::ZeroHalfLife
    );
}

#[test]
fn adversarial_zero_threshold_rejected() {
    assert_eq!(
        MigrantToken::mint(tid(1), addr(0xAA), 1_000, 100, 0, 0).unwrap_err(),
        TokenError::ZeroThreshold
    );
}

#[test]
fn adversarial_zero_half_life_in_effective_half_life_rejected() {
    assert_eq!(
        effective_half_life(0, 50, 30).unwrap_err(),
        DecayError::ZeroHalfLife
    );
}

#[test]
fn adversarial_non_owner_transfer_rejected() {
    let mut t = demo_mint(0xAA);
    let err = t.transfer(addr(0xBB), addr(0xCC), 10).unwrap_err();
    assert!(matches!(err, TokenError::NotOwner { .. }));
}

#[test]
fn adversarial_self_transfer_rejected() {
    let mut t = demo_mint(0xAA);
    assert_eq!(
        t.transfer(addr(0xAA), addr(0xAA), 5).unwrap_err(),
        TokenError::SelfTransfer
    );
}

#[test]
fn adversarial_evaporated_token_blocks_transfer() {
    // initial=4, h=1, threshold=30. At epoch 1_000 energy=0.
    let mut t = MigrantToken::mint(tid(0xFF), addr(0xAA), 4, 1, 30, 0).unwrap();
    assert!(t.is_evaporated(1_000));
    assert_eq!(
        t.transfer(addr(0xAA), addr(0xBB), 1_000).unwrap_err(),
        TokenError::Evaporated
    );
}

// ── Cross-cuts ───────────────────────────────────────────────────────────

#[test]
fn serde_round_trip_preserves_token() {
    let mut t = demo_mint(0xAA);
    t.transfer(addr(0xAA), addr(0xBB), 5).unwrap();
    let json = serde_json::to_string(&t).unwrap();
    let back: MigrantToken = serde_json::from_str(&json).unwrap();
    assert_eq!(t, back);
}

#[test]
fn transfer_result_is_deterministic() {
    let mut a = demo_mint(0xAA);
    let mut b = demo_mint(0xAA);
    let r1 = a.transfer(addr(0xAA), addr(0xBB), 50).unwrap();
    let r2 = b.transfer(addr(0xAA), addr(0xBB), 50).unwrap();
    assert_eq!(r1, r2, "transfer result must be deterministic");
    assert_eq!(a.cached_energy, b.cached_energy);
}
