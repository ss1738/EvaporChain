//! §Half-Life NFT e2e — energy-bond marketplace with three collector personas
//!
//! Scenario: three collectors hold "energy-bond" NFTs on EvaporChain.
//! LENA (loyal) holds through all 5 tiers to epoch 60_000.
//! MARCO (mid-term) holds to tier 2 then sells at epoch 5_000.
//! TURO (mercenary) flips every 1_000 epochs, always resetting to tier 0.
//!
//! The suite proves the doctrine: "loyalty literally compounds — mercenary
//! trading costs the buyer the fastest possible decay schedule, not just a
//! social reputation penalty."

use evaporchain_half_life_nft::{
    default_ladder,
    tier::{Tier, TierLadder},
    HalfLifeNft, NftError, TokenId,
};

fn alice() -> [u8; 32] {
    [0xAA; 32]
}
fn bob() -> [u8; 32] {
    [0xBB; 32]
}
fn tid(n: u8) -> TokenId {
    TokenId([n; 32])
}

fn fresh(initial: u64) -> HalfLifeNft {
    HalfLifeNft::mint(tid(1), alice(), initial, 0, default_ladder()).unwrap()
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[test]
fn five_tier_full_promotion_lifecycle() {
    // Tick through all 5 tier thresholds; confirm tier index at each boundary.
    let mut n = fresh(u64::MAX);

    // Tier 0: held < 1_000
    n.tick_to(999).unwrap();
    assert_eq!(n.current_tier_index(), 0, "999 epochs held → tier 0");

    // Tier 1: held >= 1_000
    n.tick_to(1_000).unwrap();
    assert_eq!(n.current_tier_index(), 1, "1_000 epochs held → tier 1");

    // Tier 2: held >= 5_000
    n.tick_to(5_000).unwrap();
    assert_eq!(n.current_tier_index(), 2, "5_000 epochs held → tier 2");

    // Tier 3: held >= 20_000
    n.tick_to(20_000).unwrap();
    assert_eq!(n.current_tier_index(), 3, "20_000 epochs held → tier 3");

    // Tier 4 (max): held >= 50_000
    n.tick_to(50_000).unwrap();
    assert_eq!(
        n.current_tier_index(),
        4,
        "50_000 epochs held → tier 4 (max)"
    );

    // Further time keeps tier at 4
    n.tick_to(60_000).unwrap();
    assert_eq!(
        n.current_tier_index(),
        4,
        "60_000 epochs held → still tier 4"
    );
}

#[test]
fn mercenary_cost_quantified() {
    // LENA holds continuously for 5_000 epochs (reaching tier 2, hl=400).
    // TURO transfers every 1_000 epochs (stays tier 0, hl=100 always).
    // Same initial energy. After 5_000 chain epochs, Lena must be 100×+ healthier.
    let initial: u64 = 1 << 60;

    // Lena: uninterrupted hold for 5_000 epochs
    let lena = {
        let mut t = HalfLifeNft::mint(tid(1), alice(), initial, 0, default_ladder()).unwrap();
        t.tick_to(5_000).unwrap();
        t
    };

    // Turo: flips every 1_000 epochs (4 transfers), always resetting to tier 0.
    // Holder alternates alice→bob→alice→bob→alice so each transfer is to a new holder.
    let turo = {
        let mut t = HalfLifeNft::mint(tid(2), alice(), initial, 0, default_ladder()).unwrap();
        for i in 0u64..5 {
            t.tick_to((i + 1) * 1_000).unwrap();
            if i < 4 {
                // i=0: alice→bob, i=1: bob→alice, i=2: alice→bob, i=3: bob→alice
                let next = if i % 2 == 0 { bob() } else { alice() };
                t.transfer(next).unwrap();
            }
        }
        t
    };

    assert!(
        lena.energy > turo.energy * 100,
        "LENA({}) must be 100× healthier than TURO({}) — mercenary cost is real",
        lena.energy,
        turo.energy
    );
}

#[test]
fn transfer_resets_tier_and_clock_energy_preserved() {
    // Hold to tier 2, then transfer. New holder starts at tier 0 with the
    // same (decayed) energy — the retention benefit is personal, not property-level.
    let mut n = fresh(1_000_000_000);
    n.tick_to(5_000).unwrap();
    assert_eq!(
        n.current_tier_index(),
        2,
        "should be at tier 2 after 5_000 held epochs"
    );
    let energy_before_transfer = n.energy;

    n.transfer(bob()).unwrap();
    assert_eq!(n.holder, bob());
    assert_eq!(
        n.held_epochs_by_current_holder, 0,
        "retention clock must reset to 0"
    );
    assert_eq!(n.current_tier_index(), 0, "new holder starts at tier 0");
    assert_eq!(
        n.energy, energy_before_transfer,
        "transfer must not change energy"
    );
}

#[test]
fn multiple_transfers_always_resets_to_tier_zero() {
    // 5 sequential short-hold cycles (100 epochs each), alternating alice↔bob.
    // Each transfer resets clock → holder never escapes tier 0 (threshold = 1_000).
    let mut n = fresh(1 << 40);
    let holders = [bob(), alice(), bob(), alice(), bob()];
    let mut now = 0u64;
    for &next in &holders {
        now += 100;
        n.tick_to(now).unwrap();
        n.transfer(next).unwrap();
        assert_eq!(
            n.current_tier_index(),
            0,
            "after transfer at ep={now}, must be tier 0"
        );
        assert_eq!(
            n.held_epochs_by_current_holder, 0,
            "held clock must be 0 after transfer"
        );
    }
}

#[test]
fn tier_boundary_precision_999_vs_1000() {
    // 999 held epochs → tier 0; 1000 held epochs → tier 1. Exact boundary.
    let mut at_999 = fresh(1_000_000);
    at_999.tick_to(999).unwrap();
    assert_eq!(at_999.current_tier_index(), 0, "held=999 must be tier 0");

    let mut at_1000 = fresh(1_000_000);
    at_1000.tick_to(1_000).unwrap();
    assert_eq!(at_1000.current_tier_index(), 1, "held=1000 must be tier 1");
}

#[test]
fn partial_epoch_energy_interpolation_exact() {
    // energy_at_epoch(1_000_000, hl=100, elapsed=50): exactly half a half-life.
    // Formula: after = 1M * (2*100 - 50) / (2*100) = 1M * 150/200 = 750_000.
    let mut n = fresh(1_000_000);
    n.tick_to(50).unwrap();
    assert_eq!(
        n.energy, 750_000,
        "half-life interpolation must be exact at 50/100 epochs"
    );
}

#[test]
fn one_full_half_life_halves_energy_exactly() {
    let mut n = fresh(1_000_000);
    n.tick_to(100).unwrap(); // exactly one tier-0 half-life
    assert_eq!(n.energy, 500_000, "exactly one half-life must halve energy");
}

#[test]
fn energy_monotone_non_increasing_across_all_operations() {
    // tick + transfer + tick → energy never increases
    let mut n = fresh(1_000_000_000);
    n.tick_to(500).unwrap();
    let e1 = n.energy;
    n.transfer(bob()).unwrap();
    let e2 = n.energy;
    assert_eq!(e1, e2, "transfer must not change energy");
    n.tick_to(1_000).unwrap();
    let e3 = n.energy;
    assert!(e3 <= e2, "tick after transfer must not increase energy");
    n.tick_to(5_000).unwrap();
    let e4 = n.energy;
    assert!(e4 <= e3, "continued ticking must not increase energy");
}

#[test]
fn custom_two_tier_ladder_promotion_and_decay() {
    // Custom ladder: tier 0 (hl=10, threshold=0), tier 1 (hl=100, threshold=50).
    // After 50 epochs: should be at tier 1 with slower subsequent decay.
    let ladder = TierLadder::new(vec![
        Tier {
            min_held_epochs: 0,
            half_life_epochs: 10,
        },
        Tier {
            min_held_epochs: 50,
            half_life_epochs: 100,
        },
    ])
    .unwrap();
    let mut n = HalfLifeNft::mint(tid(1), alice(), 1_000_000, 0, ladder).unwrap();

    // After 50 epochs at tier-0 (hl=10): 5 halvings → 1M / 32 = 31_250
    n.tick_to(50).unwrap();
    assert_eq!(
        n.current_tier_index(),
        1,
        "after 50 held epochs must be tier 1"
    );
    assert_eq!(n.energy, 31_250, "5 halvings of hl=10 must yield 31_250");

    // After 100 more epochs at tier-1 (hl=100): 1 halving → 31_250 / 2 = 15_625
    n.tick_to(150).unwrap();
    assert_eq!(n.energy, 15_625, "one tier-1 halving must yield 15_625");
}

#[test]
fn non_monotone_tick_rejected_with_error_details() {
    let mut n = fresh(1_000_000);
    n.tick_to(100).unwrap();

    // Equal tick
    let err = n.tick_to(100).unwrap_err();
    assert!(matches!(
        err,
        NftError::NonMonotoneTick {
            incoming: 100,
            last: 100
        }
    ));

    // Backward tick
    let err = n.tick_to(50).unwrap_err();
    assert!(matches!(
        err,
        NftError::NonMonotoneTick {
            incoming: 50,
            last: 100
        }
    ));

    // Energy and clock must not have changed
    assert_eq!(n.last_tick, 100);
}

#[test]
fn two_nfts_are_fully_independent() {
    // Separate HalfLifeNft objects share nothing; ticking one does not affect the other.
    let mut n1 = HalfLifeNft::mint(tid(1), alice(), 1_000_000, 0, default_ladder()).unwrap();
    let mut n2 = HalfLifeNft::mint(tid(2), alice(), 1_000_000, 0, default_ladder()).unwrap();

    n1.tick_to(5_000).unwrap(); // n1 promoted to tier 2
    assert_eq!(n1.current_tier_index(), 2);
    assert_eq!(n2.current_tier_index(), 0, "n2 must still be at tier 0");

    n2.transfer(bob()).unwrap();
    assert_eq!(n2.holder, bob());
    assert_eq!(
        n1.holder,
        alice(),
        "n1 holder must be unaffected by n2 transfer"
    );
}

#[test]
fn energy_fully_decays_to_zero_then_stays() {
    // Small initial energy, many halvings → reaches 0 and stays there.
    let mut n = HalfLifeNft::mint(tid(1), alice(), 1, 0, default_ladder()).unwrap();
    n.tick_to(10_000).unwrap(); // 100 halvings of hl=100 (in tier 0 until 1_000)
    assert_eq!(n.energy, 0, "energy must be 0 after sufficient halvings");

    // Further ticking keeps it at 0
    n.tick_to(20_000).unwrap();
    assert_eq!(n.energy, 0, "zero energy must stay zero");
}
