//! End-to-end integration tests for evaporchain-singh-sabi.
//!
//! Non-trivial fixture: two-token gallery — "Pilgrim" and "Hako" —
//! demonstrating decay-invariant transfer and PatinaState entropy staging.
//!
//! Token "Pilgrim" (ruined_beautiful: initial=10_000, floor=1_500/15%, half_life=10):
//!   Epoch   0: score=10_000. Pristine (all entropy axes = 0).
//!   Epoch  10 (1 half-life):
//!     decayable=8_500 → remainder=4_250. score=5_750.
//!     aged_pct=50. desaturation=50, cracks=25, foxing=25, edge_fray=12.
//!   Epoch  20 (2 half-lives):
//!     remainder=2_125. score=3_625.
//!     aged_pct=75. desaturation=75, cracks=56, foxing=56, edge_fray=42.
//!   Epoch 1000 (100 half-lives):
//!     remainder≈0. score=1_500 (floor). All axes=100.
//!
//! Token "Hako" (same params, minted at epoch 0):
//!   Transfer from Alice→Bob at epoch 5.
//!   score_at(10) IDENTICAL before and after transfer.
//!   Doctrine: "owner cannot pause; only witness."
//!
//! Arithmetic verification:
//!   energy_at_epoch(8_500, 10, 10) = 8_500 >> 1 = 4_250  (exact, no fraction)
//!   energy_at_epoch(8_500, 10, 20) = 8_500 >> 2 = 2_125  (exact)
//!   energy_at_epoch(8_500, 10, 1000) = 0  (8_500 < 2^100)
//!   aged_pct at epoch 10: (10_000-5_750)*100/8_500 = 4_250*100/8_500 = 50
//!   aged_pct at epoch 20: (10_000-3_625)*100/8_500 = 6_375*100/8_500 = 75
//!   cracks/foxing at pct=75: 75*75/100 = 5_625/100 = 56 (integer truncation)
//!   edge_fray at pct=75: 75*75*75/10_000 = 421_875/10_000 = 42 (integer truncation)
//!
//! Doctrine claim (INVENTION_STACK.md §A5.3 / Singh-Sabi):
//! "The token never evaporates — it asymptotes to ruined-beautiful (~15% floor).
//!  Owner cannot pause; only witness. Validators agree on entropy state without
//!  rendering pixels."
//!
//! Adversarial fixture: ZeroInitial, ZeroHalfLife, FloorAboveInitial,
//! NotOwner transfer, SelfTransfer, clock-skew defensive.
//!
//! INVENTION_STACK §A5.3: Singh-Sabi (Patina Tokens).

use evaporchain_singh_sabi::{
    derive_state, patina_score, PatinaError, PatinaParams, PatinaToken, TokenError,
};

// ── Helpers ───────────────────────────────────────────────────────────────

fn tid(b: u8) -> [u8; 32] {
    [b; 32]
}
fn alice() -> [u8; 32] { [0xAA; 32] }
fn bob()   -> [u8; 32] { [0xBB; 32] }

/// Pilgrim params: ruined_beautiful(10_000, 10) → floor=1_500, half_life=10.
fn pilgrim_params() -> PatinaParams {
    PatinaParams::ruined_beautiful(10_000, 10).unwrap()
}

// ── Non-trivial fixture ───────────────────────────────────────────────────

#[test]
fn pilgrim_epoch0_score_equals_initial_entropy_pristine() {
    let p = pilgrim_params();
    let score = patina_score(&p, 0, 0);
    assert_eq!(score, 10_000, "at mint score == initial");
    let st = derive_state(&p, score);
    assert_eq!(st.cracks, 0,        "pristine: no cracks");
    assert_eq!(st.desaturation, 0,  "pristine: no desaturation");
    assert_eq!(st.foxing, 0,        "pristine: no foxing");
    assert_eq!(st.edge_fray, 0,     "pristine: no edge fray");
    assert_eq!(st.score, 10_000);
}

#[test]
fn pilgrim_epoch10_one_half_life_midpoint_entropy() {
    // Decayable=8_500; energy_at_epoch(8_500,10,10)=4_250. score=5_750.
    // aged_pct=50: desaturation=50(linear), cracks=foxing=25(quadratic), edge_fray=12(cubic).
    let p = pilgrim_params();
    let score = patina_score(&p, 0, 10);
    assert_eq!(score, 5_750, "1 half-life: 1_500 + 8_500/2 = 5_750");

    let st = derive_state(&p, score);
    assert_eq!(st.desaturation, 50, "linear axis at aged_pct=50");
    assert_eq!(st.cracks,       25, "quadratic: 50^2/100 = 25");
    assert_eq!(st.foxing,       25, "quadratic: 50^2/100 = 25");
    assert_eq!(st.edge_fray,    12, "cubic: 50^3/10_000 = 12 (truncated from 12.5)");
    assert_eq!(st.score,   5_750);
}

#[test]
fn pilgrim_epoch20_two_half_lives_entropy() {
    // energy_at_epoch(8_500,10,20)=2_125. score=3_625.
    // aged_pct=75: desaturation=75, cracks=foxing=56, edge_fray=42.
    let p = pilgrim_params();
    let score = patina_score(&p, 0, 20);
    assert_eq!(score, 3_625, "2 half-lives: 1_500 + 8_500/4 = 3_625");

    let st = derive_state(&p, score);
    assert_eq!(st.desaturation, 75, "linear axis at aged_pct=75");
    assert_eq!(st.cracks,       56, "75^2/100 = 56 (truncated from 56.25)");
    assert_eq!(st.foxing,       56);
    assert_eq!(st.edge_fray,    42, "75^3/10_000 = 42 (truncated from 42.1875)");
}

#[test]
fn pilgrim_far_future_reaches_floor_not_zero() {
    // After 100 half-lives, decayable_remainder=0 → score=floor=1_500. All axes=100.
    let p = pilgrim_params();
    let score = patina_score(&p, 0, 1_000);
    assert_eq!(score, 1_500, "after 100 half-lives, score == floor");
    assert!(score > 0, "token NEVER evaporates");

    let st = derive_state(&p, score);
    assert_eq!(st.cracks,       100, "fully aged");
    assert_eq!(st.desaturation, 100);
    assert_eq!(st.foxing,       100);
    assert_eq!(st.edge_fray,    100);
}

#[test]
fn pilgrim_score_is_monotone_non_increasing() {
    let p = pilgrim_params();
    let epochs = [0u64, 5, 10, 20, 50, 100, 500, 1_000, 10_000];
    let mut prev = u64::MAX;
    for &e in &epochs {
        let score = patina_score(&p, 0, e);
        assert!(score <= prev, "score not monotone at epoch {e}: {score} > {prev}");
        prev = score;
    }
}

#[test]
fn pilgrim_floor_is_asymptote_never_crossed() {
    let p = pilgrim_params();
    let test_epochs = [0u64, 1, 10, 100, 1_000, 100_000, 10_000_000];
    for epoch in test_epochs {
        let score = patina_score(&p, 0, epoch);
        assert!(
            score >= p.floor,
            "score {score} < floor {} at epoch {epoch}",
            p.floor
        );
    }
}

#[test]
fn hako_transfer_does_not_alter_decay_curve() {
    // Doctrine: "owner cannot pause; only witness."
    // Transfer must not reset minted_at_epoch or alter the decay curve.
    let mut hako = PatinaToken::mint_ruined_beautiful(tid(1), alice(), 10_000, 10, 0).unwrap();
    let score_before = hako.score_at(10);
    hako.transfer(alice(), bob()).unwrap();
    let score_after = hako.score_at(10);
    assert_eq!(score_before, score_after, "transfer must not alter the decay curve");
    assert_eq!(hako.owner, bob());
    assert_eq!(hako.minted_at_epoch, 0, "minted_at_epoch frozen after transfer");
}

#[test]
fn witness_is_pure_function_of_epoch_regardless_of_owner() {
    // Two calls at the same epoch — with a transfer in between — return
    // the same PatinaState. Validators agree without knowing the owner.
    let mut t = PatinaToken::mint_ruined_beautiful(tid(2), alice(), 10_000, 10, 0).unwrap();
    let st_before = t.witness(10);
    t.transfer(alice(), bob()).unwrap();
    let st_after = t.witness(10);
    assert_eq!(st_before, st_after, "witness must be deterministic regardless of current owner");
}

#[test]
fn doctrine_entropy_axes_have_different_rates_at_midpoint() {
    // At aged_pct=50 the axes diverge (linear > quadratic > cubic).
    let p = pilgrim_params();
    let score = patina_score(&p, 0, 10); // → aged_pct=50
    let st = derive_state(&p, score);
    assert!(
        st.desaturation > st.cracks,
        "desaturation (linear=50) must exceed cracks (quadratic=25) at 50% aged"
    );
    assert!(
        st.cracks > st.edge_fray,
        "cracks (quadratic=25) must exceed edge_fray (cubic=12) at 50% aged"
    );
    assert_eq!(st.cracks, st.foxing, "cracks and foxing share the same quadratic curve");
}

#[test]
fn doctrine_degenerate_floor_equals_initial_never_decays() {
    // When floor == initial (decayable=0), score is constant and entropy is pristine.
    let p = PatinaParams::new(500, 500, 50).unwrap();
    for epoch in [0u64, 100, 10_000, 1_000_000] {
        let score = patina_score(&p, 0, epoch);
        assert_eq!(score, 500, "frozen token: score must stay at 500 at epoch {epoch}");
        let st = derive_state(&p, score);
        assert_eq!(st.cracks, 0,       "frozen token: always pristine at epoch {epoch}");
        assert_eq!(st.desaturation, 0, "frozen token: always pristine at epoch {epoch}");
    }
}

#[test]
fn doctrine_ruined_beautiful_floor_is_fifteen_percent() {
    let p = PatinaParams::ruined_beautiful(10_000, 100).unwrap();
    assert_eq!(p.floor, 1_500, "15% of 10_000 = 1_500");
    // floor/initial ratio: 15%.
    assert!(p.floor > 0, "floor is non-zero by construction");
    assert!(p.floor < p.initial, "floor is strictly below initial");
}

// ── Adversarial fixture ───────────────────────────────────────────────────

#[test]
fn adversarial_zero_initial_rejected() {
    assert_eq!(
        PatinaParams::new(0, 0, 100).unwrap_err(),
        PatinaError::ZeroInitial
    );
}

#[test]
fn adversarial_zero_half_life_rejected() {
    assert_eq!(
        PatinaParams::new(1_000, 100, 0).unwrap_err(),
        PatinaError::ZeroHalfLife
    );
}

#[test]
fn adversarial_floor_above_initial_rejected() {
    let err = PatinaParams::new(100, 200, 50).unwrap_err();
    assert!(matches!(err, PatinaError::FloorAboveInitial { floor: 200, initial: 100 }));
}

#[test]
fn adversarial_transfer_by_non_owner_rejected() {
    let mut t = PatinaToken::mint_ruined_beautiful(tid(3), alice(), 1_000, 100, 0).unwrap();
    let err = t.transfer(bob(), alice()).unwrap_err();
    assert!(
        matches!(err, TokenError::NotOwner { caller, owner } if caller == bob() && owner == alice()),
        "non-owner transfer must fail"
    );
}

#[test]
fn adversarial_self_transfer_rejected() {
    let mut t = PatinaToken::mint_ruined_beautiful(tid(4), alice(), 1_000, 100, 0).unwrap();
    assert_eq!(
        t.transfer(alice(), alice()).unwrap_err(),
        TokenError::SelfTransfer
    );
}

#[test]
fn adversarial_clock_skew_returns_initial() {
    // epoch_now < minted_at_epoch or epoch_now == minted_at_epoch → score=initial.
    let p = PatinaParams::new(1_000, 150, 100).unwrap();
    assert_eq!(patina_score(&p, 50, 0),  1_000, "epoch_now < minted_at → initial");
    assert_eq!(patina_score(&p, 50, 50), 1_000, "epoch_now == minted_at → initial (elapsed=0)");
}

// ── Cross-cuts ───────────────────────────────────────────────────────────

#[test]
fn serde_round_trip_preserves_token() {
    let t = PatinaToken::mint_ruined_beautiful(tid(5), alice(), 5_000, 365, 42).unwrap();
    let json = serde_json::to_string(&t).unwrap();
    let back: PatinaToken = serde_json::from_str(&json).unwrap();
    assert_eq!(t, back);
}

#[test]
fn derive_state_is_deterministic() {
    let p = pilgrim_params();
    let score = patina_score(&p, 0, 10);
    assert_eq!(
        derive_state(&p, score),
        derive_state(&p, score),
        "derive_state must be deterministic"
    );
}
