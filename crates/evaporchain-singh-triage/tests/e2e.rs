//! End-to-end integration tests for evaporchain-singh-triage.
//!
//! Non-trivial fixture: 5-item inbox across all five triage buckets.
//!
//! All items anchored at epoch_now=1_000 (last_refreshed=1_000);
//! energy_at(1_000) = energy_at_anchor for each.
//! Horizons: today=5, tomorrow=15, week=100.
//!
//!   Item D (Decayed):  energy=1,       half_life=1   → at threshold → Decayed
//!   Item T (Today):    energy=4,       half_life=1   → 2 hops → 2ep ≤ 5 → Today
//!   Item TM (Tmrw):    energy=64,      half_life=1   → 6 hops → 6ep, 5<6≤15 → Tomorrow
//!   Item W (ThisWeek): energy=131_072, half_life=1   → 17 hops → 17ep, 15<17≤100 → ThisWeek
//!   Item H (Healthy):  energy=8,       half_life=100 → 3×100=300ep > 100 → Healthy
//!
//! bucket_counts aggregation: each bucket exactly 1; total=5.
//!
//! Action fixture (applied at epoch_now=1_000):
//!   Refresh { top_up=10_000 } on Item T → new_energy=10_004 → 13 hops → Tomorrow
//!   LetDie on Item TM → item unchanged; outcome=LetDie
//!   Archive on Item W → item unchanged; outcome=Archived
//!
//! Bucket monotonicity: as epoch advances, items only move toward Decayed.
//!
//! Doctrine claim (INVENTION_STACK.md §A5.4):
//! "Wallet opens on an inbox. Buckets are pure functions of (item, epoch_now,
//! horizons) — validators agree without any clock or oracle. Swipe actions
//! have deterministic on-chain semantics. Refresh moves an item toward
//! Healthy; LetDie and Archive are non-mutating marker actions."
//!
//! INVENTION_STACK §A5.4: Singh-Triage (wallet-opens-on-inbox UX paradigm).

use evaporchain_singh_triage::{
    apply_action, bucket_counts, classify, epochs_until_threshold,
    Action, ActionError, ActionOutcome, Inbox, TriageBucket, TriageItem, TriageItemError,
};

// ── Constants ─────────────────────────────────────────────────────────────

const EPOCH: u64         = 1_000;
const H_TODAY: u64       = 5;
const H_TOMORROW: u64    = 15;
const H_WEEK: u64        = 100;

// ── Helpers ───────────────────────────────────────────────────────────────

fn item_at(byte: u8, energy: u64, half_life: u64) -> TriageItem {
    let mut id = [0u8; 32];
    id[0] = byte;
    TriageItem::new(id, energy, half_life, EPOCH).unwrap()
}

fn classify_at(it: &TriageItem) -> TriageBucket {
    classify(it, EPOCH, H_TODAY, H_TOMORROW, H_WEEK)
}

// ── Non-trivial fixture ───────────────────────────────────────────────────

#[test]
fn item_d_at_threshold_is_decayed() {
    let d = item_at(0xD0, 1, 1);
    assert_eq!(classify_at(&d), TriageBucket::Decayed,
        "energy=1 ≤ DEATH_THRESHOLD → Decayed");
    assert!(epochs_until_threshold(&d, EPOCH).is_none(),
        "at-threshold item returns None from epochs_until_threshold");
}

#[test]
fn item_t_decays_in_2_epochs_is_today() {
    let t = item_at(0xC0, 4, 1);
    // 4→2→1: 2 hops × half_life=1 = 2 epochs ≤ horizon_today=5 → Today.
    let until = epochs_until_threshold(&t, EPOCH).unwrap();
    assert_eq!(until, 2, "energy=4, half_life=1: 2 epochs to threshold");
    assert_eq!(classify_at(&t), TriageBucket::Today);
}

#[test]
fn item_tm_decays_in_6_epochs_is_tomorrow() {
    let tm = item_at(0xB0, 64, 1);
    // 64→32→16→8→4→2→1: 6 hops → 6 epochs; 5 < 6 ≤ 15 → Tomorrow.
    let until = epochs_until_threshold(&tm, EPOCH).unwrap();
    assert_eq!(until, 6, "energy=64, half_life=1: 6 epochs to threshold");
    assert_eq!(classify_at(&tm), TriageBucket::Tomorrow);
}

#[test]
fn item_w_decays_in_17_epochs_is_thisweek() {
    let w = item_at(0xA0, 131_072, 1); // 2^17 = 131_072
    // 17 hops × half_life=1 = 17 epochs; 15 < 17 ≤ 100 → ThisWeek.
    let until = epochs_until_threshold(&w, EPOCH).unwrap();
    assert_eq!(until, 17, "energy=131_072 (2^17), half_life=1: 17 epochs");
    assert_eq!(classify_at(&w), TriageBucket::ThisWeek);
}

#[test]
fn item_h_decays_in_300_epochs_is_healthy() {
    let h = item_at(0x90, 8, 100);
    // 8→4→2→1: 3 hops × half_life=100 = 300 epochs > 100 → Healthy.
    let until = epochs_until_threshold(&h, EPOCH).unwrap();
    assert_eq!(until, 300, "energy=8, half_life=100: 300 epochs to threshold");
    assert_eq!(classify_at(&h), TriageBucket::Healthy);
}

#[test]
fn bucket_counts_one_per_bucket_total_five() {
    let items = vec![
        item_at(0xD0,       1,   1),  // Decayed
        item_at(0xC0,       4,   1),  // Today
        item_at(0xB0,      64,   1),  // Tomorrow
        item_at(0xA0, 131_072,   1),  // ThisWeek
        item_at(0x90,       8, 100),  // Healthy
    ];
    let inbox = bucket_counts(&items, EPOCH, H_TODAY, H_TOMORROW, H_WEEK);

    assert_eq!(inbox.decayed,   1);
    assert_eq!(inbox.today,     1);
    assert_eq!(inbox.tomorrow,  1);
    assert_eq!(inbox.this_week, 1);
    assert_eq!(inbox.healthy,   1);
    assert_eq!(inbox.total(),   5, "all five buckets filled, total=5");
}

// ── Action fixture ────────────────────────────────────────────────────────

#[test]
fn action_refresh_moves_item_t_from_today_to_tomorrow() {
    // Before: energy=4, half_life=1 → Today (2 epochs to threshold).
    let mut t = item_at(0xC0, 4, 1);
    assert_eq!(classify_at(&t), TriageBucket::Today);

    // Refresh with 10_000: new_energy = 4 + 10_000 = 10_004.
    // epochs_until(10_004, half_life=1) ≈ 13 epochs → 5 < 13 ≤ 15 → Tomorrow.
    let outcome = apply_action(&mut t, Action::Refresh { top_up: 10_000 }, EPOCH).unwrap();
    assert!(matches!(outcome, ActionOutcome::Refreshed { new_energy: 10_004, anchored_at: 1_000 }),
        "Refresh outcome must report new_energy=10_004, anchored_at=1_000");
    assert_eq!(classify_at(&t), TriageBucket::Tomorrow,
        "refreshed Item T (10_004 energy) should be Tomorrow");
}

#[test]
fn action_let_die_is_non_mutating() {
    let mut tm = item_at(0xB0, 64, 1);
    let energy_before = tm.energy_at_anchor;
    let last_before = tm.last_refreshed_epoch;

    let outcome = apply_action(&mut tm, Action::LetDie, EPOCH).unwrap();
    assert_eq!(outcome, ActionOutcome::LetDie);
    // Item is unchanged.
    assert_eq!(tm.energy_at_anchor, energy_before, "LetDie must not mutate energy");
    assert_eq!(tm.last_refreshed_epoch, last_before, "LetDie must not mutate last_refreshed");
}

#[test]
fn action_archive_is_non_mutating() {
    let mut w = item_at(0xA0, 131_072, 1);
    let energy_before = w.energy_at_anchor;
    let last_before = w.last_refreshed_epoch;

    let outcome = apply_action(&mut w, Action::Archive, EPOCH).unwrap();
    assert_eq!(outcome, ActionOutcome::Archived);
    assert_eq!(w.energy_at_anchor, energy_before, "Archive must not mutate energy");
    assert_eq!(w.last_refreshed_epoch, last_before, "Archive must not mutate last_refreshed");
}

#[test]
fn action_refresh_reanchors_to_epoch_now() {
    let mut t = item_at(0xC0, 4, 1);
    // Apply at a later epoch.
    apply_action(&mut t, Action::Refresh { top_up: 100 }, EPOCH + 50).unwrap();
    assert_eq!(t.last_refreshed_epoch, EPOCH + 50,
        "Refresh must re-anchor item to epoch_now");
}

// ── Doctrine tests ────────────────────────────────────────────────────────

#[test]
fn doctrine_buckets_are_pure_deterministic() {
    let t = item_at(0xC0, 4, 1);
    let b1 = classify_at(&t);
    let b2 = classify_at(&t);
    assert_eq!(b1, b2, "classify must be pure and deterministic");
}

#[test]
fn doctrine_bucket_monotone_in_time() {
    // Items only move toward Decayed as epoch advances, never back.
    let order = |b: TriageBucket| match b {
        TriageBucket::Healthy => 0,
        TriageBucket::ThisWeek => 1,
        TriageBucket::Tomorrow => 2,
        TriageBucket::Today => 3,
        TriageBucket::Decayed => 4,
    };

    let item = TriageItem::new([0xF0; 32], 1_024, 10, 0).unwrap();
    let checkpoints = [0u64, 50, 100, 200, 500, 1000, 5000];
    let mut prev_order = 0;
    for &epoch in &checkpoints {
        let b = classify(&item, epoch, H_TODAY, H_TOMORROW, H_WEEK);
        assert!(order(b) >= prev_order,
            "bucket must not move toward Healthy as time passes (epoch={epoch})");
        prev_order = order(b);
    }
}

#[test]
fn doctrine_horizons_partition_the_space() {
    // With very small horizons, the same item falls in Today.
    // With very large horizons, it falls in Healthy.
    let t = item_at(0xC0, 4, 1); // epochs_until=2

    // horizons: today=5 → Today (2 ≤ 5)
    assert_eq!(classify(&t, EPOCH, 5, 10, 50), TriageBucket::Today);
    // horizons: today=1 → Tomorrow (1 < 2 ≤ 5)
    assert_eq!(classify(&t, EPOCH, 1, 5, 50), TriageBucket::Tomorrow);
    // horizons: today=1, tomorrow=1 → ThisWeek (1 < 2 ≤ 10)
    assert_eq!(classify(&t, EPOCH, 1, 1, 10), TriageBucket::ThisWeek);
    // horizons: week=1 → Healthy (1 < 2)
    assert_eq!(classify(&t, EPOCH, 1, 1, 1), TriageBucket::Healthy);
}

#[test]
fn doctrine_inbox_total_equals_item_count() {
    let items: Vec<TriageItem> = (1u8..=10).map(|i| item_at(i, 4, 1)).collect();
    let inbox = bucket_counts(&items, EPOCH, H_TODAY, H_TOMORROW, H_WEEK);
    assert_eq!(inbox.total(), 10, "inbox total must equal number of items");
}

// ── Adversarial tests ─────────────────────────────────────────────────────

#[test]
fn adversarial_zero_energy_item_rejected() {
    let err = TriageItem::new([0; 32], 0, 100, 0).unwrap_err();
    assert_eq!(err, TriageItemError::ZeroEnergy);
}

#[test]
fn adversarial_zero_half_life_item_rejected() {
    let err = TriageItem::new([0; 32], 1000, 0, 0).unwrap_err();
    assert_eq!(err, TriageItemError::ZeroHalfLife);
}

#[test]
fn adversarial_zero_top_up_refresh_rejected() {
    let mut item = item_at(0x01, 1000, 100);
    let err = apply_action(&mut item, Action::Refresh { top_up: 0 }, EPOCH).unwrap_err();
    assert_eq!(err, ActionError::ZeroTopUp);
}

#[test]
fn adversarial_empty_inbox_all_counts_zero() {
    let inbox = bucket_counts(&[], EPOCH, H_TODAY, H_TOMORROW, H_WEEK);
    assert_eq!(inbox, Inbox::default());
    assert_eq!(inbox.total(), 0);
}

#[test]
fn adversarial_classify_is_deterministic_across_calls() {
    let it = item_at(0x42, 131_072, 1);
    let b1 = classify(&it, EPOCH, H_TODAY, H_TOMORROW, H_WEEK);
    let b2 = classify(&it, EPOCH, H_TODAY, H_TOMORROW, H_WEEK);
    assert_eq!(b1, b2);
}
