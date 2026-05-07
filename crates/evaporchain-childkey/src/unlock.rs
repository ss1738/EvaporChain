//! Unlock predicate + countdown.
//!
//! `is_unlockable(letter, epoch_now)` is true iff:
//!   1. The letter is still in `Sealed` status (not opened twice).
//!   2. `epoch_now >= letter.unlock_epoch()`.
//!
//! `mark_opened` is the single state transition; idempotent only in
//! the sense that re-calling it on an already-opened letter errors.
//! Actually publishing the combined decryption key is the threshold-
//! committee's job (out of scope); this crate records the lifecycle.

use evaporchain_types::Epoch;
use thiserror::Error;

use crate::letter::{SealedLetter, SealedLetterStatus};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum UnlockError {
    #[error("letter is not yet unlockable: unlock_epoch={unlock_epoch} > now={now}")]
    NotYet { unlock_epoch: Epoch, now: Epoch },
    #[error("letter has already been opened at epoch {opened_at_epoch}")]
    AlreadyOpened { opened_at_epoch: Epoch },
}

/// True iff caller can open the letter at `epoch_now`.
pub fn is_unlockable(letter: &SealedLetter, epoch_now: Epoch) -> bool {
    letter.is_sealed() && epoch_now >= letter.unlock_epoch()
}

/// Number of epochs remaining until unlock; 0 if already unlockable.
pub fn epochs_until_unlock(letter: &SealedLetter, epoch_now: Epoch) -> u64 {
    letter.unlock_epoch().saturating_sub(epoch_now)
}

/// Transition `Sealed → Opened`. Errors if not yet unlockable or
/// already opened.
pub fn mark_opened(letter: &mut SealedLetter, epoch_now: Epoch) -> Result<(), UnlockError> {
    if let SealedLetterStatus::Opened { opened_at_epoch } = letter.status {
        return Err(UnlockError::AlreadyOpened { opened_at_epoch });
    }
    let unlock = letter.unlock_epoch();
    if epoch_now < unlock {
        return Err(UnlockError::NotYet {
            unlock_epoch: unlock,
            now: epoch_now,
        });
    }
    letter.status = SealedLetterStatus::Opened {
        opened_at_epoch: epoch_now,
    };
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::letter::SealedLetter;
    use crate::vault::{KeyShareCommitment, VaultBlob};

    fn vault() -> VaultBlob {
        let kc = KeyShareCommitment {
            validator: [0x11; 32],
        };
        VaultBlob::new([0; 32], 100, 1, vec![kc]).unwrap()
    }

    fn letter_at_18(birth: Epoch, sealed: Epoch) -> SealedLetter {
        SealedLetter::seal(
            [0; 32],
            [0xAA; 32],
            [0xBB; 32],
            birth,
            18,
            365,
            vault(),
            sealed,
        )
        .unwrap()
    }

    #[test]
    fn before_unlock_is_not_unlockable() {
        let l = letter_at_18(0, 0);
        // 18 * 365 = 6570 epochs after birth; at epoch 100 still locked.
        assert!(!is_unlockable(&l, 100));
        assert!(epochs_until_unlock(&l, 100) > 0);
    }

    #[test]
    fn at_unlock_epoch_is_unlockable() {
        let l = letter_at_18(0, 0);
        assert!(is_unlockable(&l, 18 * 365));
        assert_eq!(epochs_until_unlock(&l, 18 * 365), 0);
    }

    #[test]
    fn after_unlock_still_unlockable_until_marked() {
        let l = letter_at_18(0, 0);
        // Long after unlock, still openable until someone runs mark_opened.
        assert!(is_unlockable(&l, 100 * 365));
    }

    #[test]
    fn mark_opened_errors_before_unlock() {
        let mut l = letter_at_18(0, 0);
        let err = mark_opened(&mut l, 100).unwrap_err();
        assert!(matches!(err, UnlockError::NotYet { .. }));
        assert!(l.is_sealed(), "letter must remain sealed on failed open");
    }

    #[test]
    fn mark_opened_succeeds_at_unlock() {
        let mut l = letter_at_18(0, 0);
        mark_opened(&mut l, 18 * 365).unwrap();
        assert!(!l.is_sealed());
    }

    #[test]
    fn mark_opened_twice_errors() {
        let mut l = letter_at_18(0, 0);
        mark_opened(&mut l, 18 * 365).unwrap();
        let err = mark_opened(&mut l, 19 * 365).unwrap_err();
        assert!(matches!(err, UnlockError::AlreadyOpened { .. }));
    }

    #[test]
    fn parent_dies_seal_still_opens_on_schedule() {
        // Doctrine claim: "Parent dies? Seal still opens on schedule."
        // The letter holds no reference to the sender's liveness — the
        // unlock predicate is purely (status, unlock_epoch, now). This
        // test demonstrates the property by simulating the sender being
        // gone (we just don't reference them again).
        let l = letter_at_18(1000, 1000);
        // Sender hypothetically deceased at epoch 5000; letter still
        // valid at epoch 1000 + 18*365 = 7570.
        assert!(!is_unlockable(&l, 7569));
        assert!(is_unlockable(&l, 7570));
        // 7570 > 5000 — sender was gone for years; letter still opens.
    }

    #[test]
    fn countdown_decreases_to_zero() {
        let l = letter_at_18(0, 0);
        let unlock = l.unlock_epoch();
        for t in [0, unlock / 4, unlock / 2, unlock - 1, unlock] {
            let until = epochs_until_unlock(&l, t);
            assert_eq!(until, unlock.saturating_sub(t));
        }
    }
}
