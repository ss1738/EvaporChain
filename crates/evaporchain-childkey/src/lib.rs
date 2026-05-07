//! Singh Letter / ChildKey / Singh Vault.
//!
//! Per `research/INVENTION_STACK.md` §A5.5:
//!
//! > Parents seal text/voice/photo/video to a child, **locked by
//! > age-of-recipient (not date)**. Chain holds encrypted blob;
//! > decryption key materializes when child's verified DID reaches
//! > unlock age. Parent dies? Seal still opens on schedule.
//!
//! > **Decay synergy: inverted decay** — chain runs decay backward to
//! > compute "energy-time-to-unlock." Same primitive, opposite sign.
//! > Genuinely novel.
//!
//! > **Pitch:** *"Today Show segment writes itself."*
//! > **Singh names:** Singh Letter (primitive), ChildKey (unlock-by-age
//! > key derivation), Singh Vault (blob layer). **Build first.**
//!
//! ## What this crate ships
//!
//! Three concerns, three modules:
//!
//! - **Singh Letter** ([`letter`]) — the on-chain record:
//!   `(recipient_did, sealed_payload_hash, unlock_age_years,
//!   recipient_birth_epoch, threshold_committee, unlocked_at)`.
//! - **ChildKey** ([`unlock`]) — the unlock predicate; pure-function
//!   determination of "is the recipient old enough?" given chain time.
//!   Caller's DID layer asserts `recipient_birth_epoch`.
//! - **Singh Vault** ([`vault`]) — encrypted-blob abstraction; this
//!   crate stores only the BLAKE3 hash of ciphertext + threshold-
//!   committee set. Actual ciphertext + key shares live off-chain or
//!   in a separate blob crate.
//!
//! ## Inverted decay
//!
//! Standard decay drains energy toward zero with epoch elapsed; ChildKey
//! does the same calculation **with the sign flipped**: it tracks
//! "energy-to-unlock" that *grows* from 0 to a threshold over the
//! recipient's lifetime. Implementation-wise it's just `epoch_now -
//! recipient_birth_epoch` compared to `unlock_age_years * epochs_per_year`
//! — but framing matters: the same primitive, opposite sign, lets the
//! whitepaper claim a structural fit, not a feature glommed-on.
//!
//! ## Module map
//!
//! - [`letter`] — `SealedLetter` + lifecycle (Sealed → Opened).
//! - [`unlock`] — `is_unlockable(letter, epoch_now)` predicate +
//!   `epochs_until_unlock` countdown.
//! - [`vault`] — opaque ciphertext / key-share commitments.

pub mod letter;
pub mod unlock;
pub mod vault;

pub use letter::{LetterError, LetterId, SealedLetter, SealedLetterStatus};
pub use unlock::{epochs_until_unlock, is_unlockable, mark_opened, UnlockError};
pub use vault::{KeyShareCommitment, VaultBlob, VaultError};

#[cfg(test)]
mod press_claim_tests {
    use super::*;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "ChildKey unlocks a sealed letter when chain
    /// time exceeds `recipient_birth_epoch + unlock_age_epochs` —
    /// inverted decay (energy-to-unlock GROWS with time toward a
    /// threshold). Pre-unlock attempts to mark opened fail closed;
    /// once unlocked, the lifecycle moves Sealed → Opened
    /// idempotently."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Vault: 2-of-3 threshold over a fresh committee.
        let committee = vec![
            KeyShareCommitment {
                validator: [10u8; 32],
            },
            KeyShareCommitment {
                validator: [11u8; 32],
            },
            KeyShareCommitment {
                validator: [12u8; 32],
            },
        ];
        let vault = VaultBlob::new([0xAA; 32], 1024, 2, committee).unwrap();

        // Sealed letter: birth at epoch 0, unlocks at age 18 with
        // 100 epochs/year → unlock_epoch = 1800.
        let mut letter = SealedLetter::seal(
            [1u8; 32], // id
            [2u8; 32], // sender
            [3u8; 32], // recipient_did
            0,         // recipient_birth_epoch
            18,        // unlock_age_years
            100,       // epochs_per_year
            vault, 0, // sealed_at_epoch
        )
        .unwrap();
        assert_eq!(letter.unlock_epoch(), 1_800);

        // Pre-unlock: not unlockable; positive countdown.
        assert!(!is_unlockable(&letter, 1_000));
        assert_eq!(epochs_until_unlock(&letter, 1_000), 800);

        // Pre-unlock mark_opened fails closed.
        assert!(mark_opened(&mut letter, 1_000).is_err());
        assert!(letter.is_sealed());

        // At/after unlock_epoch: unlockable; countdown is 0.
        assert!(is_unlockable(&letter, 1_800));
        assert!(is_unlockable(&letter, 9_999));
        assert_eq!(epochs_until_unlock(&letter, 1_800), 0);

        // Mark opened succeeds; sealed → opened.
        mark_opened(&mut letter, 1_800).unwrap();
        assert!(!letter.is_sealed());

        // Zero unlock-age fails closed at construction.
        let v2 = VaultBlob::new(
            [0u8; 32],
            1,
            1,
            vec![KeyShareCommitment {
                validator: [9u8; 32],
            }],
        )
        .unwrap();
        assert!(matches!(
            SealedLetter::seal([0u8; 32], [0u8; 32], [0u8; 32], 0, 0, 100, v2, 0),
            Err(LetterError::ZeroUnlockAge)
        ));
    }
}
