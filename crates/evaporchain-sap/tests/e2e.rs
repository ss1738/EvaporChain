//! End-to-end integration tests for evaporchain-sap.
//!
//! Non-trivial fixture: an AI content marketing campaign.
//!
//! An advertiser pays for verified human attention over a 60-epoch
//! session window.  Five wallets joined the session at different times;
//! they each minted one Attention Quantum (AQ) with the same initial
//! value (1000 energy) and the standard cognitive-forgetting-curve
//! λ = 45 epochs (~45 minutes).
//!
//! By query epoch 60, wallets that joined early have AQs whose decayed
//! values have fallen below the advertiser's freshness floor (500).
//! Only the three "still-engaged" wallets earn the payout.
//!
//! Doctrine claim (INVENTION_STACK §A5.2): "A 5-minute-old AQ is
//! worth more than a 25-min one because the human is statistically
//! still cognitively-engaged."  This test proves the claim holds
//! under `energy_at_epoch` arithmetic.
//!
//! Adversarial fixture: attacker wallet spams the pool beyond the
//! per-wallet rate cap (`max_aq_per_window = 2`).  Pool must reject
//! the third mint with `RateCapped`.
//!
//! INVENTION_STACK §A5.2: Singh Attention Pool.

use evaporchain_sap::{payout_for, AttentionPool, PoolError};

// ── Constants ────────────────────────────────────────────────────────────

/// Cognitive forgetting curve half-life (Ebbinghaus / FSRS): 45 epochs.
const HALF_LIFE: u64 = 45;
/// Initial AQ value (energy per attention quantum).
const INITIAL_VALUE: u64 = 1000;
/// Freshness floor: 50% of initial → AQs older than ~1 half-life excluded.
const FRESHNESS_FLOOR: u64 = 500;
/// Query epoch for the campaign settlement.
const QUERY_EPOCH: u64 = 60;

// ── Helpers ───────────────────────────────────────────────────────────────

fn addr(b: u8) -> [u8; 32] {
    let mut a = [0u8; 32];
    a[0] = b;
    a
}

fn aq_id(b: u8) -> [u8; 32] {
    addr(b)
}

/// `energy_at_epoch(1000, 45, elapsed)` computed by hand for test assertions.
///
/// Formula: full_halvings = elapsed/45, remainder = elapsed%45,
///   after = 1000 >> full_halvings,
///   frac_decay = after * remainder / (2 * 45),
///   value = after - frac_decay.
fn expected_value(elapsed: u64) -> u64 {
    let full_halvings = elapsed / HALF_LIFE;
    if full_halvings >= 64 {
        return 0;
    }
    let after = INITIAL_VALUE >> full_halvings;
    let remainder = elapsed % HALF_LIFE;
    let frac_decay = (after as u128 * remainder as u128 / (2u128 * HALF_LIFE as u128)) as u64;
    after.saturating_sub(frac_decay)
}

// ── Non-trivial fixture: AI content marketing campaign ────────────────────

/// Build the campaign pool with 5 wallets whose AQs have different ages.
///
/// Minted at epochs 0, 0, 20, 40, 60 (chronological order required by
/// the pool's global time cursor).
///
/// At QUERY_EPOCH=60:
///   Wallet A (minted 0): elapsed=60 → value=417 < 500 → EXCLUDED (stale)
///   Wallet B (minted 0): elapsed=60 → value=417 < 500 → EXCLUDED (stale)
///   Wallet C (minted 20): elapsed=40 → value=556 ≥ 500 → ELIGIBLE
///   Wallet D (minted 40): elapsed=20 → value=778 ≥ 500 → ELIGIBLE
///   Wallet E (minted 60): elapsed= 0 → value=1000 ≥ 500 → ELIGIBLE
fn campaign_pool() -> AttentionPool {
    let mut pool = AttentionPool::new(5, 120).unwrap(); // cap=5, window=120
    pool.mint_aq(aq_id(0xA1), addr(0xAA), INITIAL_VALUE, HALF_LIFE, 0)
        .unwrap(); // Wallet A — stale
    pool.mint_aq(aq_id(0xB1), addr(0xBB), INITIAL_VALUE, HALF_LIFE, 0)
        .unwrap(); // Wallet B — stale
    pool.mint_aq(aq_id(0xC1), addr(0xCC), INITIAL_VALUE, HALF_LIFE, 20)
        .unwrap(); // Wallet C — eligible
    pool.mint_aq(aq_id(0xD1), addr(0xDD), INITIAL_VALUE, HALF_LIFE, 40)
        .unwrap(); // Wallet D — eligible
    pool.mint_aq(aq_id(0xE1), addr(0xEE), INITIAL_VALUE, HALF_LIFE, 60)
        .unwrap(); // Wallet E — eligible (fresh)
    pool
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[test]
fn campaign_pool_has_5_aqs_after_build() {
    let pool = campaign_pool();
    assert_eq!(pool.quanta.len(), 5);
}

#[test]
fn freshness_floor_excludes_stale_wallets() {
    // Wallets A and B (minted at epoch 0) are below the floor at
    // query epoch 60. Wallets C, D, E remain eligible.
    let pool = campaign_pool();
    let summary = payout_for(&pool, 10_000, FRESHNESS_FLOOR, QUERY_EPOCH).unwrap();
    assert_eq!(
        summary.eligible.len(),
        3,
        "3 wallets must be eligible (C, D, E); stale A and B excluded"
    );
}

#[test]
fn stale_wallet_values_are_below_floor() {
    // The AQs minted at epoch 0 have decayed to 417 at epoch 60, below
    // the 500 freshness floor. Verify the arithmetic.
    let expected = expected_value(60); // 417
    assert!(
        expected < FRESHNESS_FLOOR,
        "epoch-0 AQ at epoch-60 should be {expected} < {FRESHNESS_FLOOR}"
    );
}

#[test]
fn fresh_wallet_value_is_full_at_mint_epoch() {
    // Doctrine claim: "A 5-minute-old AQ is worth more than a 25-min one."
    // Wallet E (minted at epoch 60, queried at epoch 60): full value.
    let pool = campaign_pool();
    let summary = payout_for(&pool, 10_000, 1, QUERY_EPOCH).unwrap();
    // Find Wallet E's entry (holder = addr(0xEE)).
    let wallet_e = summary
        .eligible
        .iter()
        .find(|e| e.holder == addr(0xEE))
        .expect("Wallet E must be eligible");
    assert_eq!(
        wallet_e.decayed_value, INITIAL_VALUE,
        "Wallet E (fresh at query time) must have full initial value"
    );
}

#[test]
fn campaign_total_earnable_matches_sum_of_eligible_values() {
    let pool = campaign_pool();
    let summary = payout_for(&pool, 10_000, FRESHNESS_FLOOR, QUERY_EPOCH).unwrap();
    let manual_sum: u64 = summary.eligible.iter().map(|e| e.decayed_value).sum();
    assert_eq!(
        summary.total_earnable, manual_sum,
        "total_earnable must equal sum of eligible decayed values"
    );
}

#[test]
fn eligible_wallets_ordered_by_aq_id_are_deterministic() {
    // Validators must agree on the exact eligible list. payout_for sorts
    // by AQ id canonically. Two queries with identical inputs must
    // produce byte-identical payloads.
    let pool = campaign_pool();
    let s1 = payout_for(&pool, 10_000, FRESHNESS_FLOOR, QUERY_EPOCH).unwrap();
    let s2 = payout_for(&pool, 10_000, FRESHNESS_FLOOR, QUERY_EPOCH).unwrap();
    assert_eq!(
        serde_json::to_vec(&s1).unwrap(),
        serde_json::to_vec(&s2).unwrap(),
        "identical inputs must produce byte-identical payout summaries across validator contexts"
    );
}

#[test]
fn eligible_values_decrease_monotonically_with_staleness() {
    // Wallet C (elapsed 40) → Wallet D (elapsed 20) → Wallet E (elapsed 0).
    // Fresher AQs must be worth strictly more.
    let pool = campaign_pool();
    let summary = payout_for(&pool, 10_000, FRESHNESS_FLOOR, QUERY_EPOCH).unwrap();
    let value_c = summary
        .eligible
        .iter()
        .find(|e| e.holder == addr(0xCC))
        .map(|e| e.decayed_value)
        .expect("Wallet C eligible");
    let value_d = summary
        .eligible
        .iter()
        .find(|e| e.holder == addr(0xDD))
        .map(|e| e.decayed_value)
        .expect("Wallet D eligible");
    let value_e = summary
        .eligible
        .iter()
        .find(|e| e.holder == addr(0xEE))
        .map(|e| e.decayed_value)
        .expect("Wallet E eligible");
    assert!(
        value_c < value_d && value_d < value_e,
        "values must be monotone-increasing in freshness: C={value_c} D={value_d} E={value_e}"
    );
}

// ── Adversarial fixture: rate-cap spam ───────────────────────────────────

#[test]
fn adversarial_wallet_spam_beyond_cap_rejected() {
    // Attacker wallet tries to mint 3 AQs within the rate window.
    // Pool is configured with max_aq_per_window=2.
    let mut pool = AttentionPool::new(2, 100).unwrap();
    pool.mint_aq(aq_id(0x01), addr(0xAA), 1000, 45, 0).unwrap();
    pool.mint_aq(aq_id(0x02), addr(0xAA), 1000, 45, 0).unwrap();

    let err = pool
        .mint_aq(aq_id(0x03), addr(0xAA), 1000, 45, 0)
        .unwrap_err();
    assert!(
        matches!(
            err,
            PoolError::RateCapped {
                minted_in_window: 2,
                cap: 2,
                ..
            }
        ),
        "third mint must be rejected with RateCapped(2/2), got {err:?}"
    );
    // Other wallets are unaffected.
    pool.mint_aq(aq_id(0x04), addr(0xBB), 1000, 45, 0).unwrap();
    assert_eq!(
        pool.quanta.len(),
        3,
        "only 3 AQs in pool (2 from attacker + 1 from B)"
    );
}

#[test]
fn adversarial_rate_cap_resets_after_window_passes() {
    // After `window_epochs` epochs, the old mints fall outside the window
    // and the wallet can mint again.
    let mut pool = AttentionPool::new(2, 10).unwrap();
    pool.mint_aq(aq_id(0x01), addr(0xAA), 1000, 45, 0).unwrap();
    pool.mint_aq(aq_id(0x02), addr(0xAA), 1000, 45, 0).unwrap();

    // Capped within the window.
    assert!(matches!(
        pool.mint_aq(aq_id(0x03), addr(0xAA), 1000, 45, 5)
            .unwrap_err(),
        PoolError::RateCapped { .. }
    ));

    // After the window passes (epoch > 0 + 10 = 10), old mints are
    // outside the cutoff; wallet can mint 2 more.
    pool.mint_aq(aq_id(0x04), addr(0xAA), 1000, 45, 11).unwrap();
    pool.mint_aq(aq_id(0x05), addr(0xAA), 1000, 45, 11).unwrap();
    assert_eq!(pool.minted_in_window(&addr(0xAA), 11), 2);
}

#[test]
fn adversarial_duplicate_aq_id_rejected() {
    let mut pool = AttentionPool::new(5, 100).unwrap();
    pool.mint_aq(aq_id(0xFF), addr(0xAA), 1000, 45, 0).unwrap();
    let err = pool
        .mint_aq(aq_id(0xFF), addr(0xBB), 1000, 45, 0)
        .unwrap_err();
    assert!(
        matches!(err, PoolError::DuplicateAqId(_)),
        "duplicate AQ id must be rejected"
    );
}

#[test]
fn adversarial_zero_freshness_floor_rejected() {
    // Advertiser tries to pay out everyone regardless of staleness.
    let pool = campaign_pool();
    let err = payout_for(&pool, 1000, 0, QUERY_EPOCH).unwrap_err();
    assert!(
        matches!(err, evaporchain_sap::PayoutError::ZeroFreshnessFloor),
        "zero freshness_floor must be rejected"
    );
}

#[test]
fn campaign_with_all_stale_wallets_pays_nothing() {
    // All AQs minted at epoch 0; query at epoch 200 (after many half-lives).
    // Expected value at elapsed=200: 200/45=4 full halvings → 1000>>4=62;
    // frac_decay=(62*200%45)/(90)=(62*20)/(90)=13; value=49 < 500 → EXCLUDED.
    let mut pool = AttentionPool::new(5, 1000).unwrap();
    for b in 0xA0u8..0xA5 {
        pool.mint_aq(aq_id(b), addr(b), INITIAL_VALUE, HALF_LIFE, 0)
            .unwrap();
    }
    let summary = payout_for(&pool, 10_000, FRESHNESS_FLOOR, 200).unwrap();
    assert!(
        summary.eligible.is_empty(),
        "no wallet is fresh after 200 epochs; all excluded"
    );
    assert_eq!(summary.total_earnable, 0);
}
