//! §ChildKey — Singh Letter e2e (§A5.5 inverted-decay unlock primitive)
//!
//! Scenario: "Singh Letter family arc" — AMARA seals a letter to her
//! daughter ZARA at birth (epoch 0). The letter unlocks at age 18
//! (18 × 365 = 6_570 epochs). OSCAR (adversary) tries to open it
//! early; the unlock predicate rejects him. At epoch 6_570 the
//! countdown reaches zero, ZARA opens it, and the letter transitions
//! Sealed → Opened idempotently.
//!
//! The suite proves: `is_unlockable` is false before `unlock_epoch`;
//! `epochs_until_unlock` counts down correctly; `mark_opened` fails
//! before unlock; `mark_opened` succeeds at/after unlock; re-opening
//! errors with AlreadyOpened; zero-unlock-age is rejected at seal time;
//! VaultBlob threshold invariants are enforced; serde round-trip.

use evaporchain_childkey::{
    epochs_until_unlock, is_unlockable, mark_opened, KeyShareCommitment, LetterError, SealedLetter,
    UnlockError, VaultBlob, VaultError,
};

fn amara() -> [u8; 32] {
    [0xAA; 32]
}
fn zara() -> [u8; 32] {
    [0x5A; 32]
}

fn validator(b: u8) -> KeyShareCommitment {
    let mut a = [0u8; 32];
    a[0] = b;
    KeyShareCommitment { validator: a }
}

fn vault_2_of_3() -> VaultBlob {
    VaultBlob::new(
        [0xCC; 32],
        4096,
        2,
        vec![validator(0x01), validator(0x02), validator(0x03)],
    )
    .unwrap()
}

fn letter_18_years(birth_epoch: u64) -> SealedLetter {
    SealedLetter::seal(
        [0x01; 32],
        amara(),
        zara(),
        birth_epoch,
        18,  // unlock_age_years
        365, // epochs_per_year  → unlock at birth + 6_570
        vault_2_of_3(),
        birth_epoch,
    )
    .unwrap()
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[test]
fn unlock_epoch_is_birth_plus_age_times_scale() {
    let letter = letter_18_years(0);
    assert_eq!(
        letter.unlock_epoch(),
        18 * 365,
        "unlock_epoch must be 6_570"
    );
}

#[test]
fn before_unlock_not_unlockable() {
    let letter = letter_18_years(0);
    assert!(!is_unlockable(&letter, 0));
    assert!(!is_unlockable(&letter, 3_285)); // midpoint
    assert!(!is_unlockable(&letter, 6_569)); // one epoch before
}

#[test]
fn at_unlock_epoch_is_unlockable() {
    let letter = letter_18_years(0);
    assert!(is_unlockable(&letter, 6_570));
}

#[test]
fn after_unlock_still_unlockable_until_opened() {
    let letter = letter_18_years(0);
    assert!(is_unlockable(&letter, 100_000));
}

#[test]
fn countdown_decreases_to_zero() {
    let letter = letter_18_years(0);
    assert_eq!(epochs_until_unlock(&letter, 0), 6_570);
    assert_eq!(epochs_until_unlock(&letter, 3_000), 3_570);
    assert_eq!(epochs_until_unlock(&letter, 6_569), 1);
    assert_eq!(epochs_until_unlock(&letter, 6_570), 0);
    assert_eq!(epochs_until_unlock(&letter, 9_000), 0); // saturates at 0
}

#[test]
fn oscar_early_open_rejected() {
    let mut letter = letter_18_years(0);
    let err = mark_opened(&mut letter, 1_000).unwrap_err();
    assert!(
        matches!(
            err,
            UnlockError::NotYet {
                unlock_epoch: 6_570,
                now: 1_000
            }
        ),
        "early open must be rejected: {:?}",
        err
    );
    assert!(
        letter.is_sealed(),
        "letter must remain sealed after failed open"
    );
}

#[test]
fn mark_opened_succeeds_at_unlock_epoch() {
    let mut letter = letter_18_years(0);
    mark_opened(&mut letter, 6_570).unwrap();
    assert!(!letter.is_sealed());
}

#[test]
fn mark_opened_twice_errors() {
    let mut letter = letter_18_years(0);
    mark_opened(&mut letter, 6_570).unwrap();
    let err = mark_opened(&mut letter, 7_000).unwrap_err();
    assert!(
        matches!(
            err,
            UnlockError::AlreadyOpened {
                opened_at_epoch: 6_570
            }
        ),
        "double-open must error: {:?}",
        err
    );
}

#[test]
fn parent_dies_seal_still_opens_on_schedule() {
    // §A5.5 doctrine: "Parent dies? Seal still opens on schedule."
    // Unlock is purely (status, unlock_epoch, epoch_now) — no sender liveness.
    let letter = letter_18_years(1_000); // birth at epoch 1_000 → unlock at 7_570
                                         // Hypothetical: AMARA passes at epoch 4_000.
    let amara_passes = 4_000;
    // The letter is still sealed and inaccessible before unlock.
    assert!(!is_unlockable(&letter, amara_passes));
    // Long after AMARA's passing, ZARA can still open at epoch 7_570.
    assert!(is_unlockable(&letter, 7_570));
}

#[test]
fn non_zero_birth_epoch_shifts_unlock() {
    let letter = letter_18_years(10_000); // birth at 10_000 → unlock at 16_570
    assert_eq!(letter.unlock_epoch(), 16_570);
    assert!(!is_unlockable(&letter, 15_000));
    assert!(is_unlockable(&letter, 16_570));
}

#[test]
fn zero_unlock_age_rejected_at_seal() {
    let vault = vault_2_of_3();
    let err = SealedLetter::seal(
        [0x02; 32],
        amara(),
        zara(),
        0,
        0, /* zero age */
        365,
        vault,
        0,
    )
    .unwrap_err();
    assert_eq!(err, LetterError::ZeroUnlockAge);
}

#[test]
fn vault_empty_committee_rejected() {
    let err = VaultBlob::new([0; 32], 100, 1, vec![]).unwrap_err();
    assert_eq!(err, VaultError::EmptyCommittee);
}

#[test]
fn vault_zero_threshold_rejected() {
    let err = VaultBlob::new([0; 32], 100, 0, vec![validator(0x01)]).unwrap_err();
    assert_eq!(err, VaultError::ZeroThreshold);
}

#[test]
fn vault_threshold_above_committee_rejected() {
    let err = VaultBlob::new([0; 32], 100, 5, vec![validator(0x01), validator(0x02)]).unwrap_err();
    assert!(matches!(
        err,
        VaultError::ThresholdAboveCommittee { m: 5, n: 2 }
    ));
}

#[test]
fn vault_duplicate_committee_rejected() {
    let err = VaultBlob::new(
        [0; 32],
        100,
        2,
        vec![validator(0x01), validator(0x01), validator(0x02)],
    )
    .unwrap_err();
    assert_eq!(err, VaultError::DuplicateInCommittee);
}

#[test]
fn serde_round_trip() {
    let letter = letter_18_years(0);
    let json = serde_json::to_string(&letter).unwrap();
    let back: SealedLetter = serde_json::from_str(&json).unwrap();
    assert_eq!(letter, back, "SealedLetter must survive JSON round-trip");
}

#[test]
fn amara_zara_family_full_arc() {
    // Full arc: seal → countdown → Oscar fails → Zara opens at 18.
    let mut letter = letter_18_years(0);

    // Countdown at key milestones.
    assert_eq!(epochs_until_unlock(&letter, 0), 6_570);
    assert_eq!(epochs_until_unlock(&letter, 6_570 / 2), 6_570 / 2);

    // Oscar at epoch 1_000: locked.
    assert!(mark_opened(&mut letter, 1_000).is_err());
    assert!(letter.is_sealed());

    // Zara at epoch 6_570: unlocked.
    assert!(is_unlockable(&letter, 6_570));
    mark_opened(&mut letter, 6_570).unwrap();
    assert!(!letter.is_sealed());

    // Second open attempt errors.
    assert!(mark_opened(&mut letter, 7_000).is_err());
}
