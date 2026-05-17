//! End-to-end integration tests for evaporchain-singh-heartbeat.
//!
//! Non-trivial fixture: 3-item wallet progressing through four health stages.
//!
//! Alice's wallet holds three items with different decay profiles:
//!   Item A (governance token): energy=1_000_000, half_life=1000 — long-lived
//!   Item B (subscription):     energy=100,       half_life=100  — medium decay
//!   Item C (credential):       energy=4,          half_life=1   — dies by epoch 3
//!
//!   anchor: all items anchored at epoch 0 (last_refreshed=0)
//!   total_anchor = 1_000_000 + 100 + 4 = 1_000_104
//!
//!   Epoch 0 — all fresh:
//!     A=1_000_000, B=100, C=4 → total_now=1_000_104
//!     aggregate_health=1.0, worst_remaining=1.0
//!     bpm=60 (Green, arrhythmia=0)
//!
//!   Epoch 4 — C dead, A+B healthy:
//!     A: 1_000_000>>(4/1000=0)=1_000_000
//!     B: 100>>(4/100=0)=100
//!     C: 4>>(4/1=4)=0
//!     total_now=1_000_100; aggregate≈0.9999; worst_remaining=0
//!     Color=Green (aggregate≥0.75); arrhythmia≈100 (gap=0.9999)
//!     Doctrine: "wallet FEELS wrong (arrhythmia) before inbox shows it"
//!
//!   Epoch 1000 — A at half energy, B+C dead:
//!     A: 1_000_000>>(1000/1000=1)=500_000; B dead; C dead
//!     total_now=500_000; aggregate≈0.4999
//!     Color=Amber (0.40 ≤ aggregate < 0.75)
//!
//!   Epoch 3000 — A at 12.5% energy, B+C dead:
//!     A: 1_000_000>>(3000/1000=3)=125_000
//!     total_now=125_000; aggregate≈0.1249 < 0.40
//!     Color=Red; bpm near ALARMED_BPM
//!
//! Sparkline fixture: fast-decaying wallet (energy=4, half_life=1, fresh at
//! epoch=0) sampled over 24 epochs ending at epoch=23. Epochs 0..3 are Green;
//! later epochs are Red. The sparkline must trend worse over time.
//!
//! Doctrine claim (INVENTION_STACK.md §A5.4):
//! "Arrhythmia is an explicit signal — the wallet FEELS wrong before the
//! user sees the inbox. Aggregate health governs colour; worst_remaining
//! governs rhythm. A big-stake wallet with one dying item: Green colour
//! (aggregate fine) but high arrhythmia (rhythm warns you)."
//!
//! INVENTION_STACK §A5.4: Singh-Heartbeat (EvaporWallet-Pulse).

use evaporchain_singh_heartbeat::{
    color_for_health, pulse_at, sparkline_24h, PulseColor,
};
use evaporchain_singh_heartbeat::vitals::{HEALTHY_BPM, ALARMED_BPM};
use evaporchain_singh_triage::TriageItem;

// ── Helpers ───────────────────────────────────────────────────────────────

fn item(byte: u8, energy: u64, half_life: u64, last_refreshed: u64) -> TriageItem {
    let mut id = [0u8; 32];
    id[0] = byte;
    TriageItem::new(id, energy, half_life, last_refreshed).unwrap()
}

fn alice_wallet() -> Vec<TriageItem> {
    vec![
        item(0xA0, 1_000_000, 1_000, 0), // governance token
        item(0xB0,       100,   100, 0), // subscription
        item(0xC0,         4,     1, 0), // short-lived credential
    ]
}

// ── Non-trivial fixture ───────────────────────────────────────────────────

#[test]
fn epoch0_all_fresh_healthy_resting_pulse() {
    let wallet = alice_wallet();
    let v = pulse_at(&wallet, 0);

    assert_eq!(v.bpm, HEALTHY_BPM, "freshly-anchored wallet must pulse at resting 60bpm");
    assert_eq!(v.color, PulseColor::Green);
    assert_eq!(v.arrhythmia_amp, 0, "all items equally healthy → no arrhythmia");
    assert_eq!(v.aggregate_health, 1.0);
    assert_eq!(v.worst_remaining, 1.0);
}

#[test]
fn epoch4_c_dead_arrhythmia_warns_before_inbox() {
    // Doctrine: "wallet FEELS wrong (arrhythmia) before inbox shows it."
    // Item C is dead (4 half-lives in 4 epochs); A and B are still fresh.
    // Aggregate is green but arrhythmia is maximum because worst_remaining=0.
    let wallet = alice_wallet();
    let v = pulse_at(&wallet, 4);

    assert_eq!(v.color, PulseColor::Green,
        "aggregate health ≈ 0.9999 → still Green despite C dying");
    assert_eq!(v.arrhythmia_amp, 100,
        "worst_remaining=0 (C dead) while aggregate≈1.0 → max arrhythmia");
    assert_eq!(v.worst_remaining, 0.0, "Item C has zero energy at epoch 4");
    // Linear interpolation within fractional half-lives causes A to lose ~0.2%
    // in 4 epochs, so aggregate is ~0.998, not exactly 1.0.
    assert!(v.aggregate_health > 0.99, "A and B dominate: aggregate stays near 1.0");
}

#[test]
fn epoch1000_a_half_energy_b_c_dead_amber() {
    // A: 1 half-life in (1000/1000=1) → 500_000; B: 10 half-lives → 0; C: dead.
    let wallet = alice_wallet();
    let v = pulse_at(&wallet, 1000);

    assert_eq!(v.color, PulseColor::Amber,
        "aggregate≈0.4999 is in Amber zone [0.40, 0.75)");
    assert!(v.bpm > HEALTHY_BPM && v.bpm < ALARMED_BPM,
        "mid-range health → mid-range bpm, got {}", v.bpm);
}

#[test]
fn epoch3000_a_at_12_5pct_red_pulse() {
    // A: 3 half-lives → 1_000_000 >> 3 = 125_000; B: dead; C: dead.
    let wallet = alice_wallet();
    let v = pulse_at(&wallet, 3000);

    assert_eq!(v.color, PulseColor::Red,
        "aggregate≈0.125 < 0.40 → Red");
    assert!(v.bpm > 90,
        "heavily decayed wallet should pulse well above resting, got {}", v.bpm);
}

#[test]
fn health_degrades_monotonically_across_epochs() {
    // aggregate_health must be non-increasing over time.
    let wallet = alice_wallet();
    let checkpoints = [0u64, 4, 100, 500, 1000, 2000, 3000, 5000];
    let mut prev_health = f64::INFINITY;

    for &epoch in &checkpoints {
        let h = pulse_at(&wallet, epoch).aggregate_health;
        assert!(h <= prev_health,
            "health must be non-increasing: was {prev_health:.4} at prior epoch, now {h:.4} at epoch={epoch}");
        prev_health = h;
    }
}

#[test]
fn bpm_increases_monotonically_as_health_drops() {
    let wallet = alice_wallet();
    let checkpoints = [0u64, 4, 100, 500, 1000, 2000, 3000, 5000];
    let mut prev_bpm = 0u32;

    for &epoch in &checkpoints {
        let bpm = pulse_at(&wallet, epoch).bpm;
        assert!(bpm >= prev_bpm,
            "bpm must be non-decreasing over time: was {prev_bpm} at prior epoch, now {bpm} at epoch={epoch}");
        prev_bpm = bpm;
    }
}

// ── Sparkline fixture ──────────────────────────────────────────────────────

#[test]
fn sparkline_returns_24_points_for_any_wallet() {
    let pts = sparkline_24h(&alice_wallet(), 100, 5);
    assert_eq!(pts.len(), 24);
    let pts_empty = sparkline_24h(&[], 100, 1);
    assert_eq!(pts_empty.len(), 24);
}

#[test]
fn sparkline_is_chronological_newest_last() {
    let pts = sparkline_24h(&alice_wallet(), 1000, 50);
    for w in pts.windows(2) {
        assert!(w[0].epoch <= w[1].epoch,
            "sparkline must be chronological (oldest first)");
    }
    assert_eq!(pts.last().unwrap().epoch, 1000, "last point must be epoch_now");
}

#[test]
fn sparkline_shows_colour_transition_for_decaying_wallet() {
    // Single item: energy=4, half_life=1, anchored at epoch=0.
    // Epoch 0: health=1.0 → Green.
    // Epoch 23: health=0 (4>>23=0) → Red.
    // With step=1, 24 ticks span epochs [0, 23].
    let fast_dying = vec![item(0x01, 4, 1, 0)];
    let pts = sparkline_24h(&fast_dying, 23, 1);

    assert_eq!(pts.first().unwrap().vitals.color, PulseColor::Green,
        "fresh wallet at epoch=0 must be Green");
    assert_eq!(pts.last().unwrap().vitals.color, PulseColor::Red,
        "dead wallet at epoch=23 must be Red");

    // BPM must not decrease from first to last.
    assert!(pts.last().unwrap().vitals.bpm >= pts.first().unwrap().vitals.bpm,
        "BPM must be non-decreasing as wallet decays");
}

#[test]
fn sparkline_clips_at_zero_no_underflow() {
    // Wallet only 5 epochs old, requesting 24 steps of 10 → would go negative.
    let pts = sparkline_24h(&alice_wallet(), 5, 10);
    assert_eq!(pts.len(), 24, "must still return 24 points");
    assert_eq!(pts.first().unwrap().epoch, 0, "must clip at epoch=0, not underflow");
}

// ── Doctrine tests ────────────────────────────────────────────────────────

#[test]
fn doctrine_arrhythmia_is_gap_not_level() {
    // A perfectly uniform wallet (all items at same health) has arrhythmia=0
    // even when that health is low. Arrhythmia is the spread, not the level.
    let uniform_dying = vec![
        item(0x01, 4, 2, 0),
        item(0x02, 4, 2, 0),
        item(0x03, 4, 2, 0),
    ];
    // After many half-lives all are equally dead — rhythm is regular (gap=0).
    let v = pulse_at(&uniform_dying, 100);
    assert_eq!(v.arrhythmia_amp, 0,
        "uniform decay → all items same fraction → arrhythmia=0");
    // But they ARE unhealthy.
    assert_ne!(v.color, PulseColor::Green, "dead items should not be Green");
}

#[test]
fn doctrine_big_stake_one_dying_item_green_but_arrhythmic() {
    // Doctrine: "aggregate health governs colour; worst_remaining governs rhythm."
    // Giant healthy item masks the dying one in aggregate, but arrhythmia catches it.
    let wallet = vec![
        item(0x01, 1_000_000, 1_000_000, 0), // immortal giant
        item(0x02, 1_000_000, 1_000_000, 0), // immortal giant
        item(0x03,         4,           1, 0), // dying fast
    ];
    let v = pulse_at(&wallet, 4); // item 3 is dead (4 >> 4 = 0)

    assert_eq!(v.color, PulseColor::Green, "giants dominate aggregate → Green");
    assert_eq!(v.arrhythmia_amp, 100, "dying item triggers max arrhythmia");
}

#[test]
fn doctrine_color_thresholds_are_exact() {
    // Green ≥ 0.75, Amber [0.40, 0.75), Red < 0.40.
    assert_eq!(color_for_health(1.0),  PulseColor::Green);
    assert_eq!(color_for_health(0.75), PulseColor::Green);
    assert_eq!(color_for_health(0.74), PulseColor::Amber);
    assert_eq!(color_for_health(0.50), PulseColor::Amber);
    assert_eq!(color_for_health(0.40), PulseColor::Amber);
    assert_eq!(color_for_health(0.399), PulseColor::Red);
    assert_eq!(color_for_health(0.0),  PulseColor::Red);
}

#[test]
fn doctrine_bpm_bounds() {
    // BPM is always in [HEALTHY_BPM, ALARMED_BPM].
    for &epoch in &[0u64, 1, 10, 100, 1000, 5000, 100_000] {
        let v = pulse_at(&alice_wallet(), epoch);
        assert!(v.bpm >= HEALTHY_BPM && v.bpm <= ALARMED_BPM,
            "bpm={} out of [{}, {}] at epoch={epoch}", v.bpm, HEALTHY_BPM, ALARMED_BPM);
    }
}

// ── Adversarial tests ─────────────────────────────────────────────────────

#[test]
fn adversarial_empty_wallet_is_perfectly_healthy() {
    // Empty wallet: "nothing wrong because nothing to be wrong."
    let v = pulse_at(&[], 0);
    assert_eq!(v.bpm, HEALTHY_BPM);
    assert_eq!(v.color, PulseColor::Green);
    assert_eq!(v.arrhythmia_amp, 0);
    assert_eq!(v.aggregate_health, 1.0);
    assert_eq!(v.worst_remaining, 1.0);
}

#[test]
fn adversarial_empty_wallet_sparkline_all_green() {
    let pts = sparkline_24h(&[], 1000, 10);
    assert_eq!(pts.len(), 24);
    for p in &pts {
        assert_eq!(p.vitals.color, PulseColor::Green);
        assert_eq!(p.vitals.bpm, HEALTHY_BPM);
    }
}

#[test]
fn adversarial_single_item_full_decay_is_alarmed() {
    // Single item decayed to zero energy over many epochs.
    let dying = vec![item(0x01, 1, 1, 0)];
    let v = pulse_at(&dying, 100);
    assert_eq!(v.bpm, ALARMED_BPM);
    assert_eq!(v.color, PulseColor::Red);
}

#[test]
fn adversarial_step_zero_coerced_to_one() {
    // Step=0 is coerced to 1; sparkline remains chronological.
    let pts = sparkline_24h(&alice_wallet(), 50, 0);
    assert_eq!(pts.len(), 24);
    for w in pts.windows(2) {
        assert!(w[0].epoch <= w[1].epoch);
    }
}

#[test]
fn adversarial_out_of_range_health_clamps() {
    // color_for_health clamps out-of-range values to the nearest bucket.
    assert_eq!(color_for_health(-1.0), PulseColor::Red);
    assert_eq!(color_for_health(2.0),  PulseColor::Green);
    assert_eq!(color_for_health(f64::INFINITY), PulseColor::Green);
    assert_eq!(color_for_health(f64::NEG_INFINITY), PulseColor::Red);
}

#[test]
fn adversarial_pulse_is_deterministic() {
    let wallet = alice_wallet();
    let v1 = pulse_at(&wallet, 500);
    let v2 = pulse_at(&wallet, 500);
    assert_eq!(v1, v2, "pulse_at must be deterministic");
}

#[test]
fn adversarial_single_item_wallet_has_zero_arrhythmia() {
    // Only one item — aggregate equals worst → arrhythmia=0.
    let single = vec![item(0x01, 1000, 100, 0)];
    for &epoch in &[0u64, 50, 100, 200] {
        let v = pulse_at(&single, epoch);
        assert_eq!(v.arrhythmia_amp, 0,
            "single-item wallet has no spread → arrhythmia=0 at epoch={epoch}");
    }
}
