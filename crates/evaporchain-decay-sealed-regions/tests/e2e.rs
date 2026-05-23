//! End-to-end integration tests for evaporchain-decay-sealed-regions.
//!
//! Non-trivial fixture: a DeFi block-production sealing race across five
//! state segments, exercising thermal priority eviction, freeze transitions,
//! and the sub-block finality guarantee.
//!
//! Doctrine claim (INVENTION_STACK §4.3 — Decay-Sealed Regions):
//!   "Bounded nullifier sets — kills monotone privacy-chain growth."
//!   (DSR sub-claim): "Once the region's seal energy decays below the
//!   chain's finality floor, the region is cryptographically frozen —
//!   no validator can produce a competing seal over the same span.
//!   Sub-block finality is real finality."
//!
//! DeFi block-production race (height = 7):
//!
//!   Segment layout at height 7:
//!     [0,    2_000)  — DEX pool state  (hot; very high energy)
//!     [2_000, 4_000) — Governance votes (medium energy)
//!     [4_000, 6_000) — Staking state   (warm)
//!     [6_000, 8_000) — NFT registry    (cold)
//!     [8_000,10_000) — Archive         (cold)
//!
//!   Validator sealing race:
//!     Seal-P  [0,  3_000) energy=5_000  → registered first (covers DEX + partial gov)
//!     Seal-Q  [1_000, 4_000) energy=3_000 → overlaps P, lower energy → REJECTED
//!     Seal-R  [1_000, 4_000) energy=8_000 → overlaps P, higher energy → P evicted, R wins
//!     Seal-S  [4_000, 6_000) energy=2_000 → disjoint from R → accepted
//!     Seal-T  [6_000, 8_000) energy=1_500 → disjoint → accepted
//!
//!   Energy decay + freeze sweep (floor=100):
//!     Decay Seal-R: 8_000 → 50  (below floor 100) → frozen at epoch 10
//!     Decay Seal-S: 2_000 → 150 (above floor)    → stays Tentative
//!     Decay Seal-T: 1_500 → 80  (below floor)    → frozen at epoch 10
//!     freeze_below_floor(100, 10) → 2 seals frozen
//!
//!   Post-freeze: sealed regions R and T are sub-block-final.
//!     Any incoming seal overlapping R or T is rejected (OverlappingFrozenSeal),
//!     even at energy = u64::MAX.
//!
//!   Height 8 is fully independent: same spans can be registered there
//!   without any conflict with height 7.

use evaporchain_decay_sealed_regions::{
    region_commitment, Region, RegionState, RegistryError, SealRegistry,
};

// ── Fixture helpers ───────────────────────────────────────────────────────────

fn reg() -> SealRegistry { SealRegistry::new() }

fn seal(lo: u64, hi: u64, height: u64, energy: u64) -> Region {
    Region::new(lo, hi, height, [0xAA; 32], energy, 0).unwrap()
}

fn seal_root(lo: u64, hi: u64, height: u64, energy: u64, root: u8) -> Region {
    Region::new(lo, hi, height, [root; 32], energy, 0).unwrap()
}

const FLOOR:      u64 = 100;
const FREEZE_EP:  u64 = 10;
const HEIGHT:     u64 = 7;

// Spans at height 7.
const DEX_LO:  u64 = 0;
const DEX_HI:  u64 = 3_000;   // Seal-P initial span
const GOV_LO:  u64 = 1_000;   // Seal-Q / Seal-R start (overlaps DEX)
const GOV_HI:  u64 = 4_000;
const STK_LO:  u64 = 4_000;
const STK_HI:  u64 = 6_000;
const NFT_LO:  u64 = 6_000;
const NFT_HI:  u64 = 8_000;

// Energies.
const E_P: u64 = 5_000;
const E_Q: u64 = 3_000;  // < E_P → rejected
const E_R: u64 = 8_000;  // > E_P → P evicted, R wins
const E_S: u64 = 2_000;
const E_T: u64 = 1_500;

// Decay targets (post-decay values used in freeze test).
const E_R_DECAYED: u64 = 50;   // below FLOOR → frozen
const E_S_DECAYED: u64 = 150;  // above FLOOR → stays Tentative
const E_T_DECAYED: u64 = 80;   // below FLOOR → frozen

// ── Main fixture: DeFi block-production sealing race ─────────────────────────

#[test]
fn defi_block_production_sealing_race_full_lifecycle() {
    let mut r = reg();

    // ── Phase 1: sealing race ────────────────────────────────────────
    // Seal-P: first to arrive, covers [0, 3_000) at height 7.
    let p = seal_root(DEX_LO, DEX_HI, HEIGHT, E_P, 0xAA);
    let p_cmt = r.register(p.clone()).unwrap();
    assert_eq!(r.len(), 1);
    assert!(r.get(&p_cmt).is_some());

    // Seal-Q: overlaps P at [1_000, 4_000), lower energy → REJECTED.
    let q = seal_root(GOV_LO, GOV_HI, HEIGHT, E_Q, 0xBB);
    let err = r.register(q).unwrap_err();
    assert!(
        matches!(err, RegistryError::OverlappingSealHigherEnergy { incoming: E_Q, existing: E_P, .. }),
        "lower-energy Seal-Q must be rejected by thermal priority"
    );
    assert_eq!(r.len(), 1, "rejected seal must not enter registry");

    // Seal-R: overlaps P at [1_000, 4_000), HIGHER energy → P evicted, R wins.
    let rr = seal_root(GOV_LO, GOV_HI, HEIGHT, E_R, 0xCC);
    let r_cmt = r.register(rr.clone()).unwrap();
    // P must be evicted.
    assert!(r.get(&p_cmt).is_none(), "Seal-P must be evicted by higher-energy Seal-R");
    assert!(r.get(&r_cmt).is_some(), "Seal-R must be registered");
    assert_eq!(r.len(), 1);

    // Seal-S and Seal-T: disjoint from R → both accepted.
    let s = seal_root(STK_LO, STK_HI, HEIGHT, E_S, 0xDD);
    let t = seal_root(NFT_LO, NFT_HI, HEIGHT, E_T, 0xEE);
    let s_cmt = r.register(s).unwrap();
    let t_cmt = r.register(t).unwrap();
    assert_eq!(r.len(), 3, "R, S, T coexist at height {HEIGHT} (all disjoint)");

    // ── Phase 2: energy decay ────────────────────────────────────────
    r.set_energy(&r_cmt, E_R_DECAYED).unwrap(); // 50  < 100 → will freeze
    r.set_energy(&s_cmt, E_S_DECAYED).unwrap(); // 150 > 100 → stays Tentative
    r.set_energy(&t_cmt, E_T_DECAYED).unwrap(); // 80  < 100 → will freeze

    // ── Phase 3: freeze sweep ────────────────────────────────────────
    let frozen = r.freeze_below_floor(FLOOR, FREEZE_EP);
    assert_eq!(frozen, 2, "Seal-R (energy=50) and Seal-T (energy=80) must freeze");

    // Seal-S survives as Tentative (150 > 100).
    assert!(!r.get(&s_cmt).unwrap().is_frozen(), "Seal-S must remain Tentative");
    // Seal-R and Seal-T are Frozen.
    assert!(r.get(&r_cmt).unwrap().is_frozen(), "Seal-R must be Frozen");
    assert!(r.get(&t_cmt).unwrap().is_frozen(), "Seal-T must be Frozen");

    // Verify frozen_at_epoch field.
    assert!(matches!(
        r.get(&r_cmt).unwrap().state,
        RegionState::Frozen { frozen_at_epoch: FREEZE_EP }
    ));

    // ── Phase 4: finality guarantee ──────────────────────────────────
    // Any incoming seal overlapping frozen Seal-R is rejected.
    let usurper = seal_root(GOV_LO, GOV_HI + 1_000, HEIGHT, u64::MAX, 0xFF);
    let err = r.register(usurper).unwrap_err();
    assert!(
        matches!(err, RegistryError::OverlappingFrozenSeal),
        "frozen Seal-R must block all incoming seals regardless of energy"
    );

    // set_energy on a frozen seal also rejected.
    assert!(matches!(r.set_energy(&r_cmt, u64::MAX), Err(RegistryError::AlreadyFrozen)));
    assert!(matches!(r.set_energy(&t_cmt, u64::MAX), Err(RegistryError::AlreadyFrozen)));

    // ── Phase 5: different height is independent ──────────────────────
    let h8 = seal_root(GOV_LO, GOV_HI, HEIGHT + 1, E_P, 0x88);
    r.register(h8).unwrap(); // same span as frozen R but height 8 — no conflict
    assert_eq!(r.len(), 4, "height-8 seal added alongside height-7 frozen seals");
}

// ── Thermal priority: higher energy ousts lower ───────────────────────────────

#[test]
fn thermal_priority_higher_energy_ousts_lower_energy_tentative() {
    let mut r = reg();

    let weak  = seal_root(0, 100, 1, 50,  0xAA);
    let medium = seal_root(50, 150, 1, 200, 0xBB);
    let strong = seal_root(80, 120, 1, 500, 0xCC);

    let w_cmt = r.register(weak.clone()).unwrap();
    // medium overlaps weak and wins.
    let m_cmt = r.register(medium.clone()).unwrap();
    assert!(r.get(&w_cmt).is_none(), "weak must be evicted by medium");
    assert!(r.get(&m_cmt).is_some());

    // strong overlaps medium and wins.
    let s_cmt = r.register(strong).unwrap();
    assert!(r.get(&m_cmt).is_none(), "medium must be evicted by strong");
    assert!(r.get(&s_cmt).is_some());

    assert_eq!(r.len(), 1, "only the strongest seal survives in the span");
}

// ── Equal energy: existing seal wins ─────────────────────────────────────────

#[test]
fn equal_energy_existing_seal_beats_incoming() {
    let mut r = reg();
    let first  = seal_root(0, 100, 1, 500, 0xAA);
    let second = seal_root(0, 100, 1, 500, 0xBB);
    let f_cmt = r.register(first).unwrap();
    let err = r.register(second).unwrap_err();
    assert!(matches!(err, RegistryError::OverlappingSealHigherEnergy { existing: 500, incoming: 500, .. }),
        "equal energy must reject the incoming seal (existing wins)");
    assert!(r.get(&f_cmt).is_some(), "first seal must survive");
    assert_eq!(r.len(), 1);
}

// ── Freeze is a one-way transition: immutable post-freeze ────────────────────

#[test]
fn freeze_transition_is_one_way_and_irrevocable() {
    let mut r = reg();
    let s = seal_root(0, 100, 1, 50, 0xAA);
    let cmt = r.register(s).unwrap();

    // Pre-freeze: tentative, energy can be updated.
    r.set_energy(&cmt, 40).unwrap();
    assert!(!r.get(&cmt).unwrap().is_frozen());

    // Freeze.
    let frozen = r.freeze_below_floor(FLOOR, FREEZE_EP);
    assert_eq!(frozen, 1);
    assert!(r.get(&cmt).unwrap().is_frozen());

    // Post-freeze: energy update rejected.
    assert!(matches!(r.set_energy(&cmt, 1_000), Err(RegistryError::AlreadyFrozen)));

    // Post-freeze: second freeze sweep does NOT re-freeze (idempotent count).
    let second_sweep = r.freeze_below_floor(FLOOR, FREEZE_EP + 10);
    assert_eq!(second_sweep, 0, "already-frozen seals must not be double-counted");
}

// ── Frozen seal blocks all competitors including u64::MAX energy ──────────────

#[test]
fn frozen_seal_rejects_max_energy_competitor() {
    let mut r = reg();
    let s = seal_root(0, 100, 1, 5, 0xAA);
    let cmt = r.register(s).unwrap();
    r.freeze_below_floor(FLOOR, FREEZE_EP);
    assert!(r.get(&cmt).unwrap().is_frozen());

    // Maximum-possible energy incoming seal over the same span.
    let god = seal_root(0, 100, 1, u64::MAX, 0xFF);
    let err = r.register(god).unwrap_err();
    assert_eq!(err, RegistryError::OverlappingFrozenSeal,
        "frozen seal must reject even u64::MAX energy competitor — finality is real");
}

// ── Different heights are fully independent ───────────────────────────────────

#[test]
fn seals_at_different_heights_never_conflict_even_with_same_span() {
    let mut r = reg();

    // Same span at heights 0..=4: each is independent.
    for h in 0u64..=4 {
        r.register(seal(0, 100, h, 1_000)).unwrap();
    }
    assert_eq!(r.len(), 5, "5 seals at 5 different heights must all coexist");

    // Freeze one height's seal.
    for (_, s) in r.at_height(2).enumerate() {
        // Note: we can't modify during iteration; just verify they're tentative.
        assert!(!s.is_frozen(), "all seals should be tentative before freeze sweep");
    }

    // Note: cannot mutate during iteration, so just verify independence via
    // registration + count.
    r.register(seal(0, 100, 99, 1)).unwrap();
    assert_eq!(r.len(), 6);
}

// ── Decay-then-freeze chain ───────────────────────────────────────────────────

#[test]
fn energy_decay_chain_high_to_below_floor_then_frozen() {
    let mut r = reg();
    let s = seal_root(0, 100, 1, 10_000, 0xAA);
    let cmt = r.register(s).unwrap();

    // Simulate step-wise decay.
    for e in [8_000u64, 5_000, 1_000, 200, 50] {
        r.set_energy(&cmt, e).unwrap();
        assert!(!r.get(&cmt).unwrap().is_frozen(), "still Tentative at energy={e}");
    }

    // Sweep at floor=100: 50 < 100 → freeze.
    let frozen = r.freeze_below_floor(100, 99);
    assert_eq!(frozen, 1);
    assert!(r.get(&cmt).unwrap().is_frozen());

    // Cannot continue decay after freeze.
    assert!(matches!(r.set_energy(&cmt, 10), Err(RegistryError::AlreadyFrozen)));
}

// ── Disjoint seals all coexist ────────────────────────────────────────────────

#[test]
fn disjoint_seals_at_same_height_all_coexist_independently() {
    let mut r = reg();
    let spans = [(0u64, 100u64), (100, 200), (200, 300), (300, 400), (400, 500)];
    for (i, (lo, hi)) in spans.iter().enumerate() {
        r.register(seal(*lo, *hi, HEIGHT, 1_000 + i as u64 * 100)).unwrap();
    }
    assert_eq!(r.len(), 5, "5 disjoint spans must all coexist at height {HEIGHT}");

    // Adjacent spans (share boundary) are not overlapping (half-open).
    assert!(!seal(0, 100, HEIGHT, 1).overlaps(&seal(100, 200, HEIGHT, 1)),
        "adjacent [0,100) and [100,200) must NOT overlap (half-open semantics)");
}

// ── Domain-separated commitment anti-replay ───────────────────────────────────

#[test]
fn domain_tag_any_field_change_changes_commitment() {
    let base = region_commitment(10, 20, 5, &[0xAA; 32], 1_000, 0);

    // Each single-field mutation must change the commitment.
    assert_ne!(base, region_commitment(11, 20, 5, &[0xAA; 32], 1_000, 0), "span_lo changed");
    assert_ne!(base, region_commitment(10, 21, 5, &[0xAA; 32], 1_000, 0), "span_hi changed");
    assert_ne!(base, region_commitment(10, 20, 6, &[0xAA; 32], 1_000, 0), "height changed");
    assert_ne!(base, region_commitment(10, 20, 5, &[0xBB; 32], 1_000, 0), "state_root changed");
    assert_ne!(base, region_commitment(10, 20, 5, &[0xAA; 32], 1_001, 0), "energy changed");
    assert_ne!(base, region_commitment(10, 20, 5, &[0xAA; 32], 1_000, 1), "sealed_at_epoch changed");

    // BLAKE3 with domain tag ≠ raw BLAKE3 without tag (checked in unit tests,
    // confirmed here for the fixture: two seals with same region fields but
    // different domain tags would diverge).
    let with_tag = region_commitment(0, 100, 1, &[0; 32], 1_000, 0);
    let no_tag_direct: [u8; 32] = {
        let mut h = blake3::Hasher::new();
        h.update(&0u64.to_le_bytes()); // span_lo
        h.update(&100u64.to_le_bytes()); // span_hi
        h.update(&1u64.to_le_bytes()); // height
        h.update(&[0u8; 32]); // state_root
        h.update(&1_000u64.to_le_bytes()); // energy
        h.update(&0u64.to_le_bytes()); // sealed_at_epoch
        *h.finalize().as_bytes()
    };
    assert_ne!(with_tag, no_tag_direct,
        "domain-tagged commitment must differ from naive un-tagged hash");
}

// ── Freeze sweep counts correctly across mixed Tentative / Frozen ─────────────

#[test]
fn freeze_sweep_counts_only_newly_frozen_leaves_above_floor_tentative() {
    let mut r = reg();

    // 3 seals: two below floor, one above.
    let s1 = r.register(seal_root(0, 10, 1, 20, 0xAA)).unwrap();  // 20 < FLOOR=100 → freeze
    let s2 = r.register(seal_root(20, 30, 1, 60, 0xBB)).unwrap(); // 60 < FLOOR      → freeze
    let s3 = r.register(seal_root(40, 50, 1, 200, 0xCC)).unwrap();// 200 > FLOOR     → tentative

    let count = r.freeze_below_floor(FLOOR, FREEZE_EP);
    assert_eq!(count, 2, "exactly 2 seals fall below FLOOR={FLOOR}");
    assert!(r.get(&s1).unwrap().is_frozen());
    assert!(r.get(&s2).unwrap().is_frozen());
    assert!(!r.get(&s3).unwrap().is_frozen(), "above-floor seal must stay Tentative");

    // Second sweep: 0 new freezes (s1 and s2 already frozen; s3 above floor).
    let count2 = r.freeze_below_floor(FLOOR, FREEZE_EP + 5);
    assert_eq!(count2, 0, "second sweep must find 0 newly-frozen seals");
}

// ── at_height query returns only that height's seals ──────────────────────────

#[test]
fn at_height_query_filters_only_that_heights_seals() {
    let mut r = reg();

    // Populate heights 7, 8, 9 with 2 seals each.
    for h in [HEIGHT, HEIGHT + 1, HEIGHT + 2] {
        r.register(seal_root(0, 10, h, 100, h as u8)).unwrap();
        r.register(seal_root(20, 30, h, 200, h as u8 + 10)).unwrap();
    }
    assert_eq!(r.len(), 6);

    let h7_seals: Vec<_> = r.at_height(HEIGHT).collect();
    assert_eq!(h7_seals.len(), 2,
        "at_height({HEIGHT}) must return exactly 2 seals");
    assert!(h7_seals.iter().all(|s| s.height == HEIGHT),
        "all returned seals must be at height {HEIGHT}");

    // Unknown height returns 0.
    assert_eq!(r.at_height(999).count(), 0);
}

// ── Sub-block finality doctrine: frozen is real finality ─────────────────────

#[test]
fn sub_block_finality_is_real_finality_doctrine() {
    // Doctrine: "Sub-block finality is real finality."
    // The chain's hot-region sealing mechanism gives finality faster than
    // block-level BFT for state segments that need it. Once frozen, the
    // proof is: (a) no replacement possible, (b) set_energy rejected,
    // (c) the commitment is a domain-tagged BLAKE3 binding all fields,
    // so the frozen proof is also anti-replay across blocks.

    let mut r = reg();
    // Register with energy already below FLOOR (5 < 100) — no set_energy needed.
    // This isolates the state transition: energy is constant; only `state` changes.
    let hot = Region::new(0, 1_000, HEIGHT, [0xDE; 32], 5, 42).unwrap();
    let commitment_before_freeze = hot.commitment();
    let cmt = r.register(hot).unwrap();
    assert_eq!(cmt, commitment_before_freeze,
        "commitment must be deterministic (same result on repeated calls)");

    // Freeze sweep: energy=5 < FLOOR=100 → frozen.
    let frozen = r.freeze_below_floor(FLOOR, 100);
    assert_eq!(frozen, 1);

    let sealed = r.get(&cmt).unwrap();
    assert!(sealed.is_frozen(), "seal with energy below floor must be frozen");

    // The `state` field (Tentative / Frozen) is NOT part of the commitment hash.
    // Freezing alone must not change the commitment; only energy mutations would.
    assert_eq!(sealed.commitment(), commitment_before_freeze,
        "freeze transition must not change the commitment — state is excluded from the hash");

    // No validator can now produce a competing seal over any overlapping span.
    for &energy in &[1u64, FLOOR, 10_000u64, u64::MAX] {
        let competitor = Region::new(500, 1_500, HEIGHT, [0xFF; 32], energy, 200).unwrap();
        assert!(
            matches!(r.register(competitor), Err(RegistryError::OverlappingFrozenSeal)),
            "competitor with energy={energy} must be blocked by frozen seal"
        );
    }
}
