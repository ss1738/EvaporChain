//! §A5.5 — MnemoChain: FSRS-on-chain Spanish vocabulary course e2e
//!
//! Scenario: two students — ELENA (diligent, Good/Easy reviews) and
//! FELIX (inconsistent, frequent lapses) — each hold flashcard NFTs
//! in the MnemoChain Spanish deck.  The suite proves that correct
//! recall compounds stability (and therefore review interval) while
//! lapses collapse it, and that the on-chain CredentialAttestation is
//! a verifiable portable cognitive credential.

use evaporchain_mnemochain::{
    card::{Card, CardError, CardId},
    curve::{
        update_stability, Grade, SinghCurveError, MULT_AGAIN_BP, MULT_EASY_BP, MULT_GOOD_BP,
        MULT_HARD_BP, STABILITY_FLOOR,
    },
    trie::{MnemoTriePtr, TrieError},
    CredentialAttestation, ReviewOutcome,
};

// ── Actors ────────────────────────────────────────────────────────────────
fn elena() -> [u8; 32] {
    [0xE1; 32]
}
fn felix() -> [u8; 32] {
    [0xFE; 32]
}

// ── Card helpers ──────────────────────────────────────────────────────────
fn content(seed: u8) -> MnemoTriePtr {
    MnemoTriePtr::new([seed | 0x01; 32], 256).unwrap()
}
fn cid(b: u8) -> CardId {
    let mut x = [0u8; 32];
    x[0] = b;
    x
}

/// Fresh card: owner=elena, stability=10, initial_energy=1_000, minted_at=0.
fn elena_card() -> Card {
    Card::mint(cid(1), elena(), content(0xAA), 1_000, 10, 0).unwrap()
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[test]
fn spanish_course_full_review_lifecycle() {
    // Elena reviews "¿Cómo se dice 'hello'?" across 5 sessions.
    // Each Good review grows stability, extending the next interval.
    let mut card = elena_card();
    assert_eq!(card.stability, 10);
    assert_eq!(card.attempts, 0);

    // Session 1 at epoch 10 — Good: 10 * 2.5 = 25
    let o1 = card.review(elena(), Grade::Good, 10).unwrap();
    assert_eq!(o1.new_stability, 25);
    assert_eq!(card.attempts, 1);
    assert_eq!(card.correct, 1);

    // Session 2 at epoch 50 — Good: 25 * 2.5 = 62
    let o2 = card.review(elena(), Grade::Good, 50).unwrap();
    assert_eq!(o2.new_stability, 62);

    // Session 3 at epoch 200 — Easy: 62 * 4.0 = 248
    let o3 = card.review(elena(), Grade::Easy, 200).unwrap();
    assert_eq!(o3.new_stability, 248);

    // Session 4 at epoch 600 — Good: 248 * 2.5 = 620
    let o4 = card.review(elena(), Grade::Good, 600).unwrap();
    assert_eq!(o4.new_stability, 620);

    // Session 5 at epoch 2000 — Easy: 620 * 4.0 = 2480
    let o5 = card.review(elena(), Grade::Easy, 2_000).unwrap();
    assert_eq!(o5.new_stability, 2_480);

    assert_eq!(card.attempts, 5);
    assert_eq!(card.correct, 5);
    assert_eq!(card.lapses, 0);
}

#[test]
fn singh_curve_exact_multipliers() {
    // Verify every grade's exact integer arithmetic from stability=100.
    // Again:  100 * 10  / 100 = 10
    // Hard:   100 * 120 / 100 = 120
    // Good:   100 * 250 / 100 = 250
    // Easy:   100 * 400 / 100 = 400
    assert_eq!(update_stability(100, Grade::Again).unwrap(), 10);
    assert_eq!(update_stability(100, Grade::Hard).unwrap(), 120);
    assert_eq!(update_stability(100, Grade::Good).unwrap(), 250);
    assert_eq!(update_stability(100, Grade::Easy).unwrap(), 400);

    // Grade-order monotonicity
    assert!(MULT_AGAIN_BP < MULT_HARD_BP);
    assert!(MULT_HARD_BP < MULT_GOOD_BP);
    assert!(MULT_GOOD_BP < MULT_EASY_BP);
}

#[test]
fn lapse_collapses_stability_never_below_floor() {
    // Lapse on tiny stability must clamp to STABILITY_FLOOR (= 1).
    // stability=5 * 0.10 = 0 → floored to 1.
    assert_eq!(update_stability(5, Grade::Again).unwrap(), STABILITY_FLOOR);
    assert_eq!(update_stability(1, Grade::Again).unwrap(), STABILITY_FLOOR);

    // Lapse on larger stability just hurts without erasing.
    // stability=100: 100*10/100 = 10 (above floor).
    let s = update_stability(100, Grade::Again).unwrap();
    assert_eq!(s, 10);
    assert!(s >= STABILITY_FLOOR);

    // Zero stability rejects
    assert!(matches!(
        update_stability(0, Grade::Good),
        Err(SinghCurveError::ZeroStability)
    ));
}

#[test]
fn is_due_predicate_matches_stability_interval() {
    // After a Good review at epoch 0: stability = 25.
    // is_due should be false until elapsed ≥ 25.
    let mut card = elena_card(); // stability = 10
    card.review(elena(), Grade::Good, 0).unwrap(); // stability → 25

    assert!(!card.is_due(0), "epoch=0: 0 elapsed < 25 → not due");
    assert!(!card.is_due(24), "epoch=24: 24 elapsed < 25 → not due");
    assert!(card.is_due(25), "epoch=25: 25 elapsed = 25 → due");
    assert!(card.is_due(100), "epoch=100: overdue");
}

#[test]
fn energy_at_one_half_life_is_half() {
    // After exactly stability epochs, energy should be ≈ 50% of initial_energy.
    // initial_energy = 1_000, stability = 10; energy_at(10) = 500.
    let card = elena_card();
    let energy = card.energy_at(10); // elapsed = 10 = stability → one half-life
    assert_eq!(
        energy, 500,
        "energy at exactly one half-life must be initial/2"
    );

    // At 0 elapsed energy equals initial
    assert_eq!(card.energy_at(0), 1_000);

    // After two half-lives (elapsed=20): 250
    assert_eq!(card.energy_at(20), 250);
}

#[test]
fn lapse_then_recovery_stability_arc() {
    // Build up stability through Easy reviews, then lapse, then rebuild.
    let mut card = elena_card(); // stability = 10

    // Build: Easy × 2 → 10 → 40 → 160
    card.review(elena(), Grade::Easy, 10).unwrap(); // 40
    card.review(elena(), Grade::Easy, 60).unwrap(); // 160

    let pre_lapse = card.stability;
    assert_eq!(pre_lapse, 160);

    // Lapse: 160 * 0.10 = 16
    card.review(elena(), Grade::Again, 200).unwrap();
    assert_eq!(card.stability, 16);
    assert!(card.stability < pre_lapse, "lapse must collapse stability");
    assert_eq!(card.lapses, 1);

    // Recovery: Good × 2 → 16 * 2.5 = 40 → 40 * 2.5 = 100
    card.review(elena(), Grade::Good, 220).unwrap();
    assert_eq!(card.stability, 40);
    card.review(elena(), Grade::Good, 300).unwrap();
    assert_eq!(card.stability, 100);
    assert_eq!(card.correct, 4); // 2 Easy + 2 Good
    assert_eq!(card.lapses, 1);
}

#[test]
fn stability_extends_review_interval_monotonically() {
    // Successive Easy reviews must strictly grow the time before next due.
    let mut card = elena_card();
    let mut prior_stability = card.stability;
    let mut epoch = 0u64;
    for _ in 0..6 {
        epoch += prior_stability; // review exactly when due
        card.review(elena(), Grade::Easy, epoch).unwrap();
        assert!(
            card.stability > prior_stability,
            "Easy review must grow stability; was {}, now {}",
            prior_stability,
            card.stability
        );
        prior_stability = card.stability;
    }
    // After 6 Easy reviews, stability must be much larger than initial 10
    assert!(
        card.stability > 1_000,
        "6 Easy reviews from s=10 must reach >1000"
    );
}

#[test]
fn portable_credential_attestation_all_fields() {
    // CredentialAttestation captures the doctrine moat:
    // "I have provably reviewed card X across N sessions over T epochs."
    let mut card = elena_card();
    let review_epochs = [10u64, 50, 200, 800, 2000];
    let grades = [
        Grade::Good,
        Grade::Good,
        Grade::Easy,
        Grade::Again,
        Grade::Good,
    ];
    for (&ep, &g) in review_epochs.iter().zip(grades.iter()) {
        card.review(elena(), g, ep).unwrap();
    }

    let cred: CredentialAttestation = (&card).into();
    assert_eq!(cred.card_id, cid(1));
    assert_eq!(cred.owner, elena());
    assert_eq!(cred.attempts, 5);
    assert_eq!(cred.correct, 4); // Good, Good, Easy, Good = 4
    assert_eq!(cred.lapses, 1); // Again = 1
    assert_eq!(cred.last_reviewed_at_epoch, 2_000);
    assert_eq!(cred.current_stability, card.stability);

    // JSON round-trip (portable off-chain verification)
    let json = serde_json::to_string(&cred).unwrap();
    let back: CredentialAttestation = serde_json::from_str(&json).unwrap();
    assert_eq!(cred, back);
}

#[test]
fn two_students_two_cards_fully_independent() {
    // Elena and Felix each hold a separate card. Their reviews don't interfere.
    let mut elena_c = Card::mint(cid(1), elena(), content(0x11), 1_000, 10, 0).unwrap();
    let mut felix_c = Card::mint(cid(2), felix(), content(0x22), 1_000, 10, 0).unwrap();

    elena_c.review(elena(), Grade::Easy, 100).unwrap(); // s → 40
    felix_c.review(felix(), Grade::Again, 5).unwrap(); // s → 1 (floor)

    assert_eq!(elena_c.stability, 40);
    assert_eq!(felix_c.stability, 1);

    // Felix's lapses don't touch Elena's card
    assert_eq!(elena_c.lapses, 0);
    assert_eq!(felix_c.lapses, 1);

    // Elena's progress doesn't touch Felix's card
    assert_eq!(elena_c.correct, 1);
    assert_eq!(felix_c.correct, 0);
}

#[test]
fn adversarial_wrong_owner_review_rejected() {
    let mut card = elena_card();
    let err = card.review(felix(), Grade::Good, 10).unwrap_err();
    assert!(
        matches!(err, CardError::NotOwner { .. }),
        "wrong caller must produce NotOwner, got {:?}",
        err
    );
    // Card is unmodified
    assert_eq!(card.attempts, 0);
    assert_eq!(card.stability, 10);
}

#[test]
fn adversarial_backward_time_review_rejected() {
    let mut card = elena_card();
    card.review(elena(), Grade::Good, 100).unwrap(); // last_reviewed = 100

    let err = card.review(elena(), Grade::Good, 50).unwrap_err();
    assert!(
        matches!(
            err,
            CardError::ReviewBackwardsInTime {
                epoch: 50,
                last_reviewed_at: 100
            }
        ),
        "backward review must produce ReviewBackwardsInTime, got {:?}",
        err
    );
    // Card is unmodified from the successful review
    assert_eq!(card.attempts, 1);
    assert_eq!(card.last_reviewed_at_epoch, 100);
}

#[test]
fn trie_ptr_guards_empty_hash_and_zero_len() {
    // MnemoTriePtr validates its construction arguments.
    assert!(matches!(
        MnemoTriePtr::new([0u8; 32], 64).unwrap_err(),
        TrieError::EmptyHash
    ));
    assert!(matches!(
        MnemoTriePtr::new([0xAB; 32], 0).unwrap_err(),
        TrieError::ZeroLen
    ));

    // A valid ptr constructs and is queryable
    let ptr = MnemoTriePtr::new([0xCD; 32], 1024).unwrap();
    assert_eq!(ptr.content_len, 1024);
    assert_eq!(ptr.content_hash, [0xCD; 32]);
}

#[test]
fn doctrine_moat_multi_year_review_history() {
    // The headline moat: a university or employer can verify
    // "Elena has reviewed Subjuntivo card 312 times over 4 simulated years."
    let mut card = Card::mint(cid(42), elena(), content(0x42), 10_000, 10, 0).unwrap();

    // Simulate 10 spaced review sessions over 3_000 epochs
    let sessions: &[(u64, Grade)] = &[
        (10, Grade::Good),
        (35, Grade::Good),
        (100, Grade::Easy),
        (300, Grade::Good),
        (500, Grade::Hard),
        (600, Grade::Good),
        (900, Grade::Again),
        (920, Grade::Good),
        (1_500, Grade::Easy),
        (3_000, Grade::Good),
    ];
    for &(ep, g) in sessions {
        card.review(elena(), g, ep).unwrap();
    }

    let cred: CredentialAttestation = (&card).into();
    assert_eq!(cred.attempts, 10);
    assert_eq!(cred.lapses, 1); // one Again
    assert_eq!(cred.correct, 9); // nine non-Again
    assert_eq!(cred.last_reviewed_at_epoch, 3_000);
    // Stability grew substantially from initial 10
    assert!(
        cred.current_stability > 100,
        "10 mostly-correct reviews must grow stability well above 100"
    );
}
