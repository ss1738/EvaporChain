//! End-to-end integration tests for evaporchain-singh-resonance.
//!
//! Non-trivial fixture: "Beloved vs Abandoned" gallery.
//!
//!   Both tokens: initial=10_000, base_half_life=100,
//!   coupling=default_attention_curve(1000), attention_half_life=100.
//!   Minted at epoch 0.
//!
//!   Beloved:   register_engagement(1_000, 0)  — exactly at saturation.
//!   Abandoned: zero engagement throughout.
//!
//!   At epoch 0 (just minted, before engagement):
//!     attention=0 → h_eff=50 (min_scale=50bp, decay 2× faster than base).
//!
//!   After register_engagement(1_000, 0) on Beloved:
//!     attention=1_000 (saturation) → h_eff=100 (mid_scale=100bp = base rate).
//!     cached_energy re-anchored to 10_000 at epoch 0.
//!
//!   At epoch 100:
//!     Beloved: attention_at(100) = energy_at_epoch(1_000, 100, 100) = 500.
//!       scale = 50 + 50*500/1000 = 75. h_eff = 75.
//!       energy = energy_at_epoch(10_000, 75, 100):
//!         full=1, rem=25. after=5_000. frac=5_000*25/150=833. result=4_167.
//!     Abandoned: attention=0. h_eff=50.
//!       energy = energy_at_epoch(10_000, 50, 100):
//!         full=2, rem=0. after=2_500. result=2_500.
//!     Beloved(4_167) >> Abandoned(2_500).
//!
//!   "Yesterday's likes evaporate":
//!     Register massive engagement (10_000) at epoch 0.
//!     h_eff_at(0) = 730 (piecewise scale near max).
//!     At epoch 100_000: attention decays to 0. h_eff slides toward 50.
//!     h_eff_at(100_000) < h_eff_at(0). Token is mortal.
//!
//!   Anti-Black-Mirror guard:
//!     Adversarial sustained engagement cannot push h_eff to max_scale × base.
//!     max_possible = base × 800/100 = 800. h_eff < 800 always.
//!
//! Doctrine claim (INVENTION_STACK.md §A5.3 / Singh-Resonance):
//! "Loved art slows toward immortality; ignored art accelerates toward
//!  zero. Yesterday's likes evaporate on the token too — a firework of
//!  engagement followed by silence still leads to death. Slows TOWARD
//!  immortality, never pins to it (Black-Mirror guard)."
//!
//! Adversarial fixture: ZeroInitial, ZeroBaseHalfLife, ZeroEngagement,
//! NotOwner, SelfTransfer, Evaporated.
//!
//! INVENTION_STACK §A5.3: Singh-Resonance (Vital-Sign NFTs).

use evaporchain_singh_resonance::{
    effective_half_life, CouplingParams, ResonanceToken, TokenError, TokenId,
};

// ── Helpers ───────────────────────────────────────────────────────────────

fn tid(b: u8) -> TokenId {
    [b; 32]
}
fn addr(b: u8) -> [u8; 32] {
    [b; 32]
}

fn doctrine_coupling() -> CouplingParams {
    CouplingParams::default_attention_curve(1_000).unwrap()
}

fn gallery_token(creator: u8) -> ResonanceToken {
    ResonanceToken::mint(
        tid(creator),
        addr(creator),
        10_000,
        100,
        doctrine_coupling(),
        100,
        0,
    )
    .unwrap()
}

// ── Non-trivial fixture ───────────────────────────────────────────────────

#[test]
fn fixture_beloved_vs_abandoned_at_epoch100() {
    let mut beloved = gallery_token(0xA0);
    let abandoned = gallery_token(0xB0);

    // Beloved receives engagement at saturation at epoch 0.
    beloved.register_engagement(1_000, 0).unwrap();

    let e_beloved = beloved.energy_at(100).unwrap();
    let e_abandoned = abandoned.energy_at(100).unwrap();

    assert_eq!(
        e_abandoned, 2_500,
        "ignored: energy_at_epoch(10_000,50,100)=2_500"
    );
    assert_eq!(
        e_beloved, 4_167,
        "loved: energy_at_epoch(10_000,75,100)=4_167"
    );
    assert!(
        e_beloved > e_abandoned,
        "beloved ({e_beloved}) must outlive abandoned ({e_abandoned})"
    );
}

#[test]
fn fixture_zero_attention_gives_min_scale_half_life() {
    // Doctrine: newly minted (attention=0) → h_eff = base × min_scale/100 = base/2.
    let t = gallery_token(0xC0);
    let h = t.effective_half_life_at(0).unwrap();
    assert_eq!(h, 50, "ignored token: base=100, min_scale=50bp → h_eff=50");
}

#[test]
fn fixture_saturation_engagement_gives_base_rate() {
    // At attention=1_000 (= saturation): h_eff = base × mid_scale/100 = base.
    let mut t = gallery_token(0xC1);
    t.register_engagement(1_000, 0).unwrap();
    let h = t.effective_half_life_at(0).unwrap();
    assert_eq!(h, 100, "at saturation: h_eff == base_half_life");
}

#[test]
fn fixture_yesterdays_likes_evaporate() {
    // Massive burst at epoch 0, then silence. Far in the future, h_eff slides
    // back toward min (token becomes mortal again).
    let mut t = gallery_token(0xC2);
    t.register_engagement(10_000, 0).unwrap();
    let h_now = t.effective_half_life_at(0).unwrap();
    let h_later = t.effective_half_life_at(100_000).unwrap();
    assert!(
        h_later < h_now,
        "far-future h_eff ({h_later}) must be less than post-burst ({h_now})"
    );
    // Eventually collapses toward min (50).
    assert!(
        h_later <= 100,
        "after 100_000 epochs of silence, attention should be near 0"
    );
}

#[test]
fn fixture_sustained_engagement_beats_silent_across_lifecycle() {
    // Beloved gets periodic engagement; abandoned gets none. Track at 500 epochs.
    let mut sustained = gallery_token(0xD0);
    let abandoned = gallery_token(0xD1);

    // Register every 50 epochs: epochs 0, 50, 100, ..., 450.
    for ep in (0u64..500).step_by(50) {
        sustained.register_engagement(500, ep).unwrap();
    }

    let e_sustained = sustained.energy_at(500).unwrap();
    let e_abandoned = abandoned.energy_at(500).unwrap();

    assert!(
        e_sustained > e_abandoned,
        "sustained ({e_sustained}) must outlive abandoned ({e_abandoned}) at epoch 500"
    );
}

// ── Doctrine tests ────────────────────────────────────────────────────────

#[test]
fn doctrine_anti_black_mirror_engagement_cannot_pin_immortality() {
    // Max effective half-life ceiling = base × max_scale/100 = 100 × 800/100 = 800.
    let p = doctrine_coupling();
    let max_ceiling = 100 * 800 / 100; // 800
    for adversarial_attention in [10_000u64, 1_000_000, u64::MAX / 2] {
        let h = effective_half_life(100, adversarial_attention, &p).unwrap();
        assert!(
            h < max_ceiling,
            "adversarial attention={adversarial_attention} produced h_eff={h} ≥ ceiling {max_ceiling}"
        );
    }
}

#[test]
fn doctrine_effective_half_life_is_monotone_in_attention() {
    let p = doctrine_coupling();
    let attention_levels = [0u64, 100, 500, 1_000, 5_000, 100_000];
    let mut prev_h = 0u64;
    for &attn in &attention_levels {
        let h = effective_half_life(100, attn, &p).unwrap();
        assert!(
            h >= prev_h,
            "h_eff not monotone at attention={attn}: {h} < {prev_h}"
        );
        prev_h = h;
    }
}

#[test]
fn doctrine_engagement_reanchor_never_inflates_energy() {
    // Doctrine: register_engagement re-anchors at the *current* energy.
    // This means engagement cannot increase the token's energy, only
    // slow its future decay.
    let mut t = gallery_token(0xE0);
    let energy_before = t.energy_at(0).unwrap();
    t.register_engagement(1_000, 0).unwrap();
    let energy_after = t.cached_energy;
    // Re-anchor uses energy_at(0) which is computed BEFORE the engagement boost.
    assert_eq!(
        energy_before, energy_after,
        "re-anchor must not inflate energy — it captures energy as-of-now"
    );
}

#[test]
fn doctrine_scale_at_zero_attention_is_exactly_min() {
    let p = doctrine_coupling();
    let h = effective_half_life(200, 0, &p).unwrap();
    assert_eq!(
        h, 100,
        "zero attention: base=200, min_scale=50bp → h_eff=100"
    );
}

// ── Adversarial fixture ───────────────────────────────────────────────────

#[test]
fn adversarial_zero_initial_energy_rejected() {
    let err =
        ResonanceToken::mint(tid(1), addr(0xAA), 0, 100, doctrine_coupling(), 100, 0).unwrap_err();
    assert_eq!(err, TokenError::ZeroInitial);
}

#[test]
fn adversarial_zero_base_half_life_rejected() {
    let err = ResonanceToken::mint(tid(1), addr(0xAA), 1_000, 0, doctrine_coupling(), 100, 0)
        .unwrap_err();
    assert_eq!(err, TokenError::ZeroBaseHalfLife);
}

#[test]
fn adversarial_zero_engagement_weight_rejected() {
    let mut t = gallery_token(0xAA);
    assert_eq!(
        t.register_engagement(0, 0).unwrap_err(),
        TokenError::ZeroEngagement
    );
}

#[test]
fn adversarial_non_owner_transfer_rejected() {
    let mut t = gallery_token(0xAA);
    let err = t.transfer(addr(0xBB), addr(0xCC), 5).unwrap_err();
    assert!(matches!(err, TokenError::NotOwner { .. }));
}

#[test]
fn adversarial_self_transfer_rejected() {
    let mut t = gallery_token(0xAA);
    assert_eq!(
        t.transfer(addr(0xAA), addr(0xAA), 5).unwrap_err(),
        TokenError::SelfTransfer
    );
}

#[test]
fn adversarial_evaporated_token_blocks_transfer() {
    // Tiny initial + small base half-life + min scale → dies fast.
    let t = ResonanceToken::mint(tid(0xEE), addr(0xAA), 4, 1, doctrine_coupling(), 100, 0).unwrap();
    assert!(
        t.is_evaporated(10_000),
        "token should be dead at epoch 10_000"
    );
    let mut t2 = t;
    assert_eq!(
        t2.transfer(addr(0xAA), addr(0xBB), 10_000).unwrap_err(),
        TokenError::Evaporated
    );
}

// ── Cross-cuts ───────────────────────────────────────────────────────────

#[test]
fn serde_round_trip_preserves_token() {
    let mut t = gallery_token(0xAA);
    t.register_engagement(500, 5).unwrap();
    let json = serde_json::to_string(&t).unwrap();
    let back: ResonanceToken = serde_json::from_str(&json).unwrap();
    assert_eq!(t, back);
}

#[test]
fn energy_at_is_deterministic() {
    let t = gallery_token(0xAA);
    assert_eq!(
        t.energy_at(100).unwrap(),
        t.energy_at(100).unwrap(),
        "energy_at must be deterministic"
    );
}
