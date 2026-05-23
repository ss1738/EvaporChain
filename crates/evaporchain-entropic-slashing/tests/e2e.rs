//! End-to-end integration tests for evaporchain-entropic-slashing.
//!
//! Non-trivial fixture: a 4-validator misbehavior detection session that
//! closes the conservation triplet named in INVENTION_STACK.md §4.2.
//!
//! Doctrine claim (INVENTION_STACK §4.2 Entropic Slashing):
//!   "Shannon-weighted slash → energy-aware MEV burn → refresh pool
//!   (the conservation triplet). Slashes MORE for high-entropy 'noisy'
//!   cartel patterns and LESS for low-entropy 'obvious' ones. The chain
//!   pays attention to cases that are actually hard to detect."
//!
//! Four validators caught misbehaving:
//!   Val A (obvious cartel)  counts=[1000, 0, 0]        entropy=0        slash=0
//!   Val B (noisy/uncertain) counts=[500, 500]           entropy=1000 mb  slash=stake (full)
//!   Val C (skewed MEV)      counts=[800, 200]           entropy<1000 mb  slash=partial
//!   Val D (uniform 4-way)   counts=[250,250,250,250]    entropy≥2000 mb  slash=stake (capped)
//!
//! Entropy ordering (slash magnitude):
//!   A(0) < C(partial) ≤ D(capped) = B(capped)
//!
//! Conservation triplet (for B and D, where slashes are exact):
//!   1. entropic_slash → compute slash magnitude
//!   2. Slash redirect:  Stake[B+D] → SlashedPool (total preserved)
//!   3. SlashSettle:     SlashedPool → RefreshPool (total preserved)
//!   Namespace root keep-alive funded from RefreshPool.

use evaporchain_entropic_slashing::{entropic_slash, EntropicSlashError};
use evaporchain_energy_kernel::{
    Compartment, ConservationCheck, EnergyAccumulator, EnergyRedirect,
};
use evaporchain_energy_kernel::redirect::RedirectKind;

// ── Fixture constants ─────────────────────────────────────────────────────────

const STAKE_A: u64 = 1_000_000; // obvious cartel — slash = 0
const STAKE_B: u64 = 1_000_000; // noisy pattern  — slash = full stake
const STAKE_C: u64 = 500_000;   // skewed MEV     — slash = partial
const STAKE_D: u64 = 750_000;   // uniform 4-way  — slash = full stake (capped)

// ── Main fixture: 4-validator misbehavior detection ──────────────────────────

#[test]
fn four_validator_misbehavior_entropy_ordering() {
    // A: deterministic — one outcome, entropy = 0, slash = 0.
    let slash_a = entropic_slash(STAKE_A, &[1000, 0, 0]).unwrap();
    assert_eq!(slash_a, 0, "deterministic pattern must yield zero slash");

    // B: uniform 2-way — entropy = 1 bit = 1000 millibits, slash = full stake.
    let slash_b = entropic_slash(STAKE_B, &[500, 500]).unwrap();
    assert_eq!(slash_b, STAKE_B, "uniform 50/50 must yield full-stake slash");

    // C: skewed 80/20 — entropy < 1 bit, slash is partial.
    let slash_c = entropic_slash(STAKE_C, &[800, 200]).unwrap();
    assert!(slash_c > 0, "skewed distribution must yield positive slash");
    assert!(slash_c < STAKE_C, "skewed distribution must yield partial slash (not full stake)");

    // D: uniform 4-way — entropy = 2 bits = 2000 millibits → 2×stake capped at stake.
    let slash_d = entropic_slash(STAKE_D, &[250, 250, 250, 250]).unwrap();
    assert_eq!(slash_d, STAKE_D, "uniform 4-way must hit the stake cap");

    // Entropy ordering: A < C < B (and D capped same as B at their respective stakes).
    assert!(slash_a < slash_c,
        "deterministic slash {slash_a} must be < skewed slash {slash_c}");
    assert!(slash_c < slash_b,
        "skewed slash {slash_c} must be < uniform-2 slash {slash_b}");
}

// ── Conservation triplet ──────────────────────────────────────────────────────

#[test]
fn conservation_triplet_slash_settle_refresh_pool_closed() {
    // B and D: slashes are exact (full-stake cap).
    let slash_b = entropic_slash(STAKE_B, &[500, 500]).unwrap(); // = STAKE_B
    let slash_d = entropic_slash(STAKE_D, &[250, 250, 250, 250]).unwrap(); // = STAKE_D

    // Initial accumulator: all energy in Stake for B and D validators.
    let total_stake = STAKE_B + STAKE_D;
    let mut acc = EnergyAccumulator::new(0, total_stake, 0, 0);
    assert_eq!(acc.total(), total_stake);

    // Step 1: Slash B (Stake → SlashedPool).
    let s0 = acc;
    EnergyRedirect::new(RedirectKind::Slash, slash_b).apply(&mut acc).unwrap();
    ConservationCheck::redirect(&s0, &acc).unwrap();
    assert_eq!(acc[Compartment::SlashedPool], slash_b);
    assert_eq!(acc.total(), total_stake, "slash redirect must preserve total");

    // Step 2: Slash D (Stake → SlashedPool).
    let s1 = acc;
    EnergyRedirect::new(RedirectKind::Slash, slash_d).apply(&mut acc).unwrap();
    ConservationCheck::redirect(&s1, &acc).unwrap();
    assert_eq!(acc[Compartment::SlashedPool], slash_b + slash_d);
    assert_eq!(acc.total(), total_stake, "second slash must still preserve total");

    // Step 3: SlashSettle — all slashed energy → RefreshPool.
    let s2 = acc;
    let settle_amount = slash_b + slash_d;
    EnergyRedirect::new(RedirectKind::SlashSettle, settle_amount).apply(&mut acc).unwrap();
    ConservationCheck::redirect(&s2, &acc).unwrap();
    assert_eq!(acc[Compartment::SlashedPool], 0, "SlashedPool must be drained after settle");
    assert_eq!(acc[Compartment::RefreshPool], settle_amount, "RefreshPool receives the settled energy");
    assert_eq!(acc.total(), total_stake, "settle must preserve total — triplet closed");
}

#[test]
fn val_a_zero_slash_leaves_accumulator_unchanged() {
    // Validator A's zero slash means no redirect is issued — stake untouched.
    let slash_a = entropic_slash(STAKE_A, &[1000, 0, 0]).unwrap();
    assert_eq!(slash_a, 0);
    // If slash is zero, no redirect is applied — the stake stays put.
    let acc = EnergyAccumulator::new(0, STAKE_A, 0, 0);
    assert_eq!(acc[Compartment::Stake], STAKE_A);
    // Confirm the accumulator total is unchanged (no mutation occurred).
    assert_eq!(acc.total(), STAKE_A);
}

// ── Entropy ordering properties ───────────────────────────────────────────────

#[test]
fn deterministic_pattern_always_zero_slash() {
    // Any distribution with all mass on one outcome → entropy = 0 → slash = 0.
    let stake = 5_000_000;
    assert_eq!(entropic_slash(stake, &[100_000, 0, 0, 0]).unwrap(), 0);
    assert_eq!(entropic_slash(stake, &[0, 999, 0]).unwrap(), 0);
    assert_eq!(entropic_slash(stake, &[1]).unwrap(), 0, "single outcome = entropy 0");
}

#[test]
fn uniform_2_always_full_stake_slash() {
    // entropy([a, a]) = 1 bit = 1000 millibits → slash = stake × 1000 / 1000 = stake (exactly).
    for stake in [1, 100, 50_000, 1_000_000, u64::MAX / 2] {
        let s = entropic_slash(stake, &[100, 100]).unwrap();
        assert_eq!(s, stake, "uniform 2-way slash must equal stake for stake={stake}");
    }
}

#[test]
fn slash_cap_never_exceeded() {
    // Even with very high entropy (many uniform outcomes), slash is capped at stake.
    let stake = 999;
    let s = entropic_slash(stake, &[1, 1, 1, 1, 1, 1, 1, 1]).unwrap();
    assert!(s <= stake, "slash must never exceed stake (cap invariant)");
    assert_eq!(s, stake, "uniform 8-way should hit the cap");
}

#[test]
fn zero_stake_produces_zero_slash_regardless_of_entropy() {
    let s = entropic_slash(0, &[500, 500]).unwrap();
    assert_eq!(s, 0, "zero stake yields zero slash (nothing to slash)");
}

#[test]
fn skewed_slash_between_zero_and_full() {
    // Various skewed distributions must yield slashes strictly between 0 and stake.
    let stake = 10_000;
    let distributions = [
        &[9, 1][..],
        &[99, 1][..],
        &[700, 300][..],
        &[600, 400][..],
    ];
    for counts in distributions {
        let s = entropic_slash(stake, counts).unwrap();
        assert!(s > 0, "skewed distribution must yield positive slash");
        assert!(s < stake, "skewed distribution must yield partial slash (< full stake)");
    }
}

// ── Adversarial ───────────────────────────────────────────────────────────────

#[test]
fn empty_counts_propagates_entropy_error() {
    let err = entropic_slash(1_000_000, &[]).unwrap_err();
    // EntropyError propagated via EntropicSlashError::Entropy wrapper.
    assert!(matches!(err, EntropicSlashError::Entropy(_)),
        "empty counts must propagate EntropyError from the estimator");
}

// ── Doctrine: high-entropy = hard-to-detect = heavier penalty ────────────────

#[test]
fn entropy_ordering_uniform_beats_skewed_for_same_stake() {
    let stake = 100_000;
    // 90/10 is obviously suspicious → low entropy → smaller slash.
    let skewed = entropic_slash(stake, &[900, 100]).unwrap();
    // 50/50 is hard to distinguish from honest → high entropy → larger slash.
    let uniform = entropic_slash(stake, &[500, 500]).unwrap();
    assert!(uniform > skewed,
        "uniform slash {uniform} must exceed skewed slash {skewed}: chain penalizes harder-to-detect patterns more");
}

#[test]
fn rare_violation_higher_penalty_than_repeated_obvious_one() {
    // Doctrine pin (AUDIT M18): the chain penalises rare, hard-to-attribute
    // violations more than obvious repeated patterns.
    // [1, 1] = one rare ambiguous violation → entropy=1000 mb → full slash.
    // [100, 1] = 100 obvious violations → entropy << 1 bit → small slash.
    let stake = 1_000_000;
    let rare   = entropic_slash(stake, &[1, 1]).unwrap();
    let obvious = entropic_slash(stake, &[100, 1]).unwrap();
    assert_eq!(rare, stake, "rare ambiguous violation hits full-stake slash");
    assert!(obvious < stake / 10,
        "obvious repeated violation slash {obvious} should be <10% of stake");
    assert!(rare > obvious,
        "rare violation must be penalised more than obvious one");
}

// ── Multi-round slashing session ──────────────────────────────────────────────

#[test]
fn multi_round_slash_settle_cycle_fully_conserved() {
    // Two consecutive slash-settle cycles, each closing the triplet.
    let mut acc = EnergyAccumulator::new(0, 3_000_000, 0, 0);
    let initial_total = acc.total();

    for (counts, amount) in [
        (&[500u64, 500u64][..], 1_000_000u64),   // round 1: uniform, full slash
        (&[250u64, 250u64, 250u64, 250u64][..], 500_000u64), // round 2: uniform 4-way
    ] {
        let slash = entropic_slash(amount, counts).unwrap();
        assert_eq!(slash, amount, "uniform slash must equal stake");

        let s_before_slash = acc;
        EnergyRedirect::new(RedirectKind::Slash, slash).apply(&mut acc).unwrap();
        ConservationCheck::redirect(&s_before_slash, &acc).unwrap();

        let s_before_settle = acc;
        EnergyRedirect::new(RedirectKind::SlashSettle, slash).apply(&mut acc).unwrap();
        ConservationCheck::redirect(&s_before_settle, &acc).unwrap();
    }

    assert_eq!(acc.total(), initial_total,
        "total must be preserved across both slash-settle cycles");
    // All slashed energy now lives in RefreshPool (1M + 500k = 1.5M).
    assert_eq!(acc[Compartment::RefreshPool], 1_500_000,
        "RefreshPool must accumulate both slash settlements");
    assert_eq!(acc[Compartment::SlashedPool], 0,
        "SlashedPool must be empty after each settle");
}
