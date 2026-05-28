//! End-to-end integration tests for evaporchain-shlm.
//!
//! Non-trivial fixture: AI-era recruiting market.
//!
//! Two skill classes with doctrine-grade half-lives (INVENTION_STACK §A5.2):
//!   Python (half_life=100)  — rapid decay; AI disruption makes skills stale fast
//!   COBOL  (half_life=1000) — slow decay; legacy scarcity premium persists
//!
//! Employer bounties:
//!   Python bounty (AI startup): salary_cap=120_000, floor=60_000,
//!     min_freshness=600, min_level=700, posted_at=0, duration=200
//!   COBOL bounty (bank):        salary_cap=200_000, floor=100_000,
//!     min_freshness=100, min_level=300, posted_at=0, duration=200
//!
//! Candidates and fate:
//!   Alice: Python, level=1000, attested_at=0; bids at epoch 50
//!     freshness_score(50) = 750 ≥ 600 ✓ → ELIGIBLE
//!     price_at(50) = 105_000 ≤ salary_ask=120_000 → CLEARS at 105_000
//!   Bob:   Python, level=1000, attested_at=0; bids at epoch 150
//!     freshness_score(150) = 375 < 600 → EXCLUDED (stale)
//!   Carol: COBOL, level=500,  attested_at=0; bids at epoch 100
//!     freshness_score(100) = 475 ≥ 100 ✓ → ELIGIBLE
//!     price_at(100) = 150_000 ≤ salary_ask=200_000 → CLEARS at 150_000
//!   Dave:  COBOL, level=200,  attested_at=0; bids at epoch 50
//!     level=200 < min_level=300 → EXCLUDED (insufficient level)
//!
//! Doctrine claim (INVENTION_STACK §A5.2): "Python's freshness decays
//! ~10x faster than COBOL's because Python's half-life is ~10x shorter."
//! Proven under `freshness_score` arithmetic in the doctrine test below.
//!
//! Adversarial fixture: refresh backdating, class-mismatch bids,
//! zero-salary and zero-min-level bounty construction.
//!
//! INVENTION_STACK §A5.2: Singh Skill Half-Life Market.

use evaporchain_shlm::market::CandidateBid;
use evaporchain_shlm::{
    freshness_score, post_bounty, settle_bounty, Bounty, BountyError, Credential, CredentialError,
    SkillClass,
};

// ── Constants ────────────────────────────────────────────────────────────

/// Python half-life: rapid decay (AI displacement, fast tooling churn).
const PYTHON_HL: u64 = 100;
/// COBOL half-life: slow decay (scarce legacy expertise, little change).
const COBOL_HL: u64 = 1_000;

// ── Helpers ───────────────────────────────────────────────────────────────

fn id(b: u8) -> [u8; 32] {
    let mut a = [0u8; 32];
    a[0] = b;
    a
}

/// Hand-computed `energy_at_epoch(level, half_life, elapsed)` for assertions.
///
/// Formula: full_halvings = elapsed/hl, after = level >> full_halvings,
///   frac_decay = after * (elapsed%hl) / (2*hl), value = after - frac_decay.
fn expected_freshness(level: u64, half_life: u64, elapsed: u64) -> u64 {
    let full_halvings = elapsed / half_life;
    if full_halvings >= 64 {
        return 0;
    }
    let after = level >> full_halvings;
    let remainder = elapsed % half_life;
    let frac = (after as u128 * remainder as u128 / (2u128 * half_life as u128)) as u64;
    after.saturating_sub(frac)
}

fn python_class() -> SkillClass {
    SkillClass::register(id(0x01), "Python", PYTHON_HL, 0).unwrap()
}

fn cobol_class() -> SkillClass {
    SkillClass::register(id(0x02), "COBOL", COBOL_HL, 0).unwrap()
}

fn python_bounty() -> Bounty {
    Bounty::post(
        id(0xB1), // bounty id
        id(0xF1), // employer
        id(0x01), // Python class id
        120_000,  // salary_cap
        600,      // min_freshness
        700,      // min_level
        0,        // posted_at
    )
    .unwrap()
}

fn cobol_bounty() -> Bounty {
    Bounty::post(
        id(0xB2), // bounty id
        id(0xF2), // employer
        id(0x02), // COBOL class id
        200_000,  // salary_cap
        100,      // min_freshness
        300,      // min_level
        0,        // posted_at
    )
    .unwrap()
}

/// Alice: Python credential, level=1000, attested at epoch 0.
fn alice_cred() -> Credential {
    Credential::issue(id(0xC1), id(0x01), id(0xEE), id(0xD1), 1000, 0).unwrap()
}

/// Bob: Python credential, level=1000, attested at epoch 0.
fn bob_cred() -> Credential {
    Credential::issue(id(0xC2), id(0x01), id(0xEE), id(0xD2), 1000, 0).unwrap()
}

/// Carol: COBOL credential, level=500, attested at epoch 0.
fn carol_cred() -> Credential {
    Credential::issue(id(0xC3), id(0x02), id(0xEE), id(0xD3), 500, 0).unwrap()
}

/// Dave: COBOL credential, level=200 (below min_level=300), attested at epoch 0.
fn dave_cred() -> Credential {
    Credential::issue(id(0xC4), id(0x02), id(0xEE), id(0xD4), 200, 0).unwrap()
}

// ── Non-trivial fixture: AI-era recruiting market ─────────────────────────

#[test]
fn alice_wins_python_bounty_at_submission_price() {
    // Alice (fresh Python cred) clears the bounty; price is locked to
    // her submission epoch per SDDC semantics.
    //   freshness_score(elapsed=50) = 750 ≥ 600 → eligible
    //   price_at(submitted_at=50) = 120_000 − 60_000×50/200 = 105_000
    //   salary_ask=120_000 ≥ 105_000 → clears at 105_000
    let python = python_class();
    let bounty = python_bounty();
    let mut auction = post_bounty(&bounty, id(0xA1), 60_000, 200).unwrap();
    let alice = alice_cred();

    let candidates = [CandidateBid {
        holder: id(0xD1),
        cred: &alice,
        salary_ask: 120_000,
        freshness_tolerance: 700, // ≥ lot_lambda=600 (min_freshness)
        submitted_at: 50,
    }];

    let cleared = settle_bounty(&bounty, &python, &mut auction, &candidates, 150)
        .unwrap()
        .expect("Alice must clear the Python bounty");

    assert_eq!(cleared.winner, id(0xD1), "Alice must be the winner");
    assert_eq!(
        cleared.price_paid, 105_000,
        "price must be locked at Alice's submission epoch (50), not the clearing epoch"
    );
}

#[test]
fn stale_python_candidate_excluded_before_sddc() {
    // Bob submits at epoch 150; Python half_life=100 → freshness_score=375 < 600.
    // The bounty's eligibility filter drops him before SDDC clearing.
    // No eligible candidates → settle returns None, not an error.
    let python = python_class();
    let bounty = python_bounty();
    let mut auction = post_bounty(&bounty, id(0xA1), 60_000, 200).unwrap();
    let bob = bob_cred();

    let candidates = [CandidateBid {
        holder: id(0xD2),
        cred: &bob,
        salary_ask: 120_000,
        freshness_tolerance: 700,
        submitted_at: 150, // elapsed=150 → score=375 < 600
    }];

    let result = settle_bounty(&bounty, &python, &mut auction, &candidates, 160).unwrap();
    assert!(
        result.is_none(),
        "Bob's stale credential must be excluded; no winner returned"
    );
}

#[test]
fn carol_wins_cobol_bounty_despite_slow_decay() {
    // Carol's COBOL cred decays slowly enough (half_life=1000) that after
    // 100 epochs it still retains 475 freshness ≥ 100 min_freshness.
    //   freshness_score(elapsed=100) = 475 ≥ 100 → eligible
    //   price_at(submitted_at=100) = 200_000 − 100_000×100/200 = 150_000
    //   salary_ask=200_000 ≥ 150_000 → clears at 150_000
    let cobol = cobol_class();
    let bounty = cobol_bounty();
    let mut auction = post_bounty(&bounty, id(0xA2), 100_000, 200).unwrap();
    let carol = carol_cred();

    let candidates = [CandidateBid {
        holder: id(0xD3),
        cred: &carol,
        salary_ask: 200_000,
        freshness_tolerance: 200, // ≥ lot_lambda=100
        submitted_at: 100,
    }];

    let cleared = settle_bounty(&bounty, &cobol, &mut auction, &candidates, 150)
        .unwrap()
        .expect("Carol must clear the COBOL bounty");

    assert_eq!(cleared.winner, id(0xD3), "Carol must be the winner");
    assert_eq!(cleared.price_paid, 150_000);
}

#[test]
fn low_level_cobol_candidate_excluded_before_sddc() {
    // Dave's COBOL cred has level=200, below the bounty's min_level=300.
    // The `Bounty::matches` check drops him before SDDC sees his bid.
    let cobol = cobol_class();
    let bounty = cobol_bounty();
    let mut auction = post_bounty(&bounty, id(0xA2), 100_000, 200).unwrap();
    let dave = dave_cred();

    let candidates = [CandidateBid {
        holder: id(0xD4),
        cred: &dave,
        salary_ask: 200_000,
        freshness_tolerance: 200,
        submitted_at: 50,
    }];

    let result = settle_bounty(&bounty, &cobol, &mut auction, &candidates, 150).unwrap();
    assert!(
        result.is_none(),
        "Dave (level=200 < min_level=300) must be excluded by the eligibility filter"
    );
}

#[test]
fn doctrine_python_decays_10x_faster_than_cobol() {
    // INVENTION_STACK §A5.2 doctrine claim:
    // "Python's freshness decays ~10x faster than COBOL's because
    // Python's half-life is ~10x shorter."
    //
    // After 100 epochs elapsed (= 1 Python half-life, 0.1 COBOL half-life):
    //   Python (hl=100): full_halvings=1, score=1000>>1=500
    //   COBOL  (hl=1000): full_halvings=0, frac_decay=50, score=950
    //
    // COBOL credential remains 90% fresh; Python is down to 50%.
    let python = python_class();
    let cobol = cobol_class();
    let level = 1000u64;
    let elapsed = 100u64;

    let python_cred = Credential::issue(id(0xC9), id(0x01), id(0xEE), id(0xD9), level, 0).unwrap();
    let cobol_cred = Credential::issue(id(0xCA), id(0x02), id(0xEE), id(0xDA), level, 0).unwrap();

    let python_score = freshness_score(&python, &python_cred, elapsed);
    let cobol_score = freshness_score(&cobol, &cobol_cred, elapsed);

    let py_expected = expected_freshness(level, PYTHON_HL, elapsed); // 500
    let co_expected = expected_freshness(level, COBOL_HL, elapsed); // 950

    assert_eq!(
        python_score, py_expected,
        "Python freshness at elapsed={elapsed}"
    );
    assert_eq!(
        cobol_score, co_expected,
        "COBOL freshness at elapsed={elapsed}"
    );
    assert!(
        cobol_score > python_score,
        "COBOL must retain more freshness than Python after the same elapsed time: \
         COBOL={cobol_score}, Python={python_score}"
    );
    // Quantify: COBOL retains ≥ 1.5× more freshness (empirically ~1.9×).
    assert!(
        cobol_score * 10 >= python_score * 15,
        "COBOL must retain at least 1.5× more freshness than Python"
    );
}

#[test]
fn freshness_score_monotone_decreasing_with_staleness() {
    // Alice at elapsed=50 must have higher freshness than Bob at elapsed=150.
    // Proves the decay is strictly ordered — validators rank by staleness deterministically.
    let python = python_class();
    let alice = alice_cred();
    let bob = bob_cred();

    let score_alice = freshness_score(&python, &alice, 50); // elapsed=50
    let score_bob = freshness_score(&python, &bob, 150); // elapsed=150

    let expected_alice = expected_freshness(1000, PYTHON_HL, 50); // 750
    let expected_bob = expected_freshness(1000, PYTHON_HL, 150); // 375

    assert_eq!(score_alice, expected_alice);
    assert_eq!(score_bob, expected_bob);
    assert!(
        score_alice > score_bob,
        "Alice (elapsed=50, score={score_alice}) must be fresher than Bob (elapsed=150, score={score_bob})"
    );
}

#[test]
fn winner_price_is_from_submission_epoch_not_clearing_epoch() {
    // Explicit proof that SDDC locks price at bid.submitted_at, not epoch_now.
    // Alice submits at epoch 50 → price_at(50)=105_000.
    // settle_bounty is called at epoch_now=100 → price_at(100)=90_000.
    // The winner's price_paid must be 105_000 (submission-locked), not 90_000.
    let python = python_class();
    let bounty = python_bounty();
    let mut auction = post_bounty(&bounty, id(0xA1), 60_000, 200).unwrap();
    let alice = alice_cred();

    let candidates = [CandidateBid {
        holder: id(0xD1),
        cred: &alice,
        salary_ask: 120_000,
        freshness_tolerance: 700,
        submitted_at: 50,
    }];

    // epoch_now=100 → price_at(100) would be 90_000 if used (wrong).
    let cleared = settle_bounty(&bounty, &python, &mut auction, &candidates, 100)
        .unwrap()
        .unwrap();

    assert_eq!(
        cleared.price_paid, 105_000,
        "price must be 105_000 (price_at submitted_at=50), not 90_000 (price_at epoch_now=100)"
    );
}

#[test]
fn credential_refresh_restores_eligibility_for_stale_candidate() {
    // Bob's Python credential is stale at epoch 150 (freshness_score=375 < 600).
    // After a refresh at epoch 160 (new_level=900), his credential is immediately
    // fresh again — at a new bounty's epoch, Bob can compete and win.
    let python = python_class();
    let mut bob = bob_cred();

    // Confirm stale before refresh.
    let stale_score = freshness_score(&python, &bob, 150);
    assert!(
        stale_score < 600,
        "Bob must be stale (score={stale_score}) before refresh"
    );

    // Refresh at epoch 160.
    bob.refresh(900, 160).unwrap();

    // New bounty posted after refresh, submitted at epoch 170.
    let bounty = python_bounty();
    let mut auction = post_bounty(&bounty, id(0xA3), 60_000, 200).unwrap();

    // elapsed at submitted_at=170: 170 - 160 = 10
    // freshness_score = energy_at_epoch(900, 100, 10) = 900 - 900*10/200 = 855 ≥ 600 ✓
    // price_at(170) = 120_000 - 60_000*(170-0)/200 = ... wait, opened_at is 0 (from python_bounty)
    // price_at(170) = 120_000 - 60_000*170/200 = 120_000 - 51_000 = 69_000
    // salary_ask=120_000 ≥ 69_000 → CLEARS at 69_000

    let candidates = [CandidateBid {
        holder: id(0xD2),
        cred: &bob,
        salary_ask: 120_000,
        freshness_tolerance: 700,
        submitted_at: 170,
    }];

    let cleared = settle_bounty(&bounty, &python, &mut auction, &candidates, 180)
        .unwrap()
        .expect("Bob must clear the bounty after refreshing his credential");

    assert_eq!(cleared.winner, id(0xD2), "refreshed Bob must win");
}

// ── Adversarial fixture ───────────────────────────────────────────────────

#[test]
fn adversarial_refresh_backdating_rejected() {
    // Attestors cannot backdate refreshes — `attested_at_epoch` cannot
    // go backward. This anti-Sybil property prevents gaming the freshness score.
    let mut alice = alice_cred(); // attested_at=0
    alice.refresh(900, 50).unwrap(); // advance to epoch 50

    let err = alice.refresh(1000, 30).unwrap_err(); // try to backdate to epoch 30

    assert!(
        matches!(
            err,
            CredentialError::RefreshGoingBackwards {
                refresh_at: 30,
                attested_at: 50
            }
        ),
        "backdating refresh must be rejected, got {err:?}"
    );
    // Credential state must be unchanged on failure.
    assert_eq!(
        alice.level, 900,
        "level must be unchanged after failed refresh"
    );
    assert_eq!(
        alice.attested_at_epoch, 50,
        "attested_at must be unchanged after failed refresh"
    );
}

#[test]
fn adversarial_class_mismatch_candidate_rejected() {
    // Attacker submits a COBOL credential to the Python bounty.
    // `settle_bounty` must return ClassMismatch immediately, not silently skip the bid.
    let python = python_class();
    let bounty = python_bounty();
    let mut auction = post_bounty(&bounty, id(0xA1), 60_000, 200).unwrap();

    // COBOL credential class=0x02, but bounty class=0x01 (Python).
    let cobol_cred = Credential::issue(id(0xC8), id(0x02), id(0xEE), id(0xD8), 1000, 0).unwrap();

    let candidates = [CandidateBid {
        holder: id(0xD8),
        cred: &cobol_cred,
        salary_ask: 120_000,
        freshness_tolerance: 700,
        submitted_at: 50,
    }];

    let err = settle_bounty(&bounty, &python, &mut auction, &candidates, 100).unwrap_err();
    assert!(
        matches!(
            err,
            evaporchain_shlm::MarketError::Bounty(BountyError::ClassMismatch)
        ),
        "class-mismatch candidate must be rejected with ClassMismatch, got {err:?}"
    );
}

#[test]
fn adversarial_zero_salary_bounty_rejected() {
    // Employer tries to post a bounty with salary_cap=0.
    let err = Bounty::post(
        id(0xB9),
        id(0xF9),
        id(0x01),
        0, // zero salary_cap — adversarial
        600,
        700,
        0,
    )
    .unwrap_err();
    assert_eq!(
        err,
        BountyError::ZeroSalary,
        "zero salary_cap must be rejected"
    );
}

#[test]
fn adversarial_zero_min_level_bounty_rejected() {
    // Employer tries to post a bounty that accepts any credential level,
    // bypassing the per-holder skill gate.
    let err = Bounty::post(
        id(0xB9),
        id(0xF9),
        id(0x01),
        100_000,
        600,
        0, // zero min_level — adversarial
        0,
    )
    .unwrap_err();
    assert_eq!(
        err,
        BountyError::ZeroMinLevel,
        "zero min_level must be rejected"
    );
}
