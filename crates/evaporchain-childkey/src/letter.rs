//! `SealedLetter` — the on-chain record. Status transitions
//! `Sealed` → `Opened` only; never re-sealed.

use evaporchain_types::{AccountAddress, Epoch};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::vault::VaultBlob;

pub type LetterId = [u8; 32];

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LetterError {
    #[error("unlock_age_years must be > 0")]
    ZeroUnlockAge,
    #[error("epochs_per_year must be > 0")]
    ZeroEpochsPerYear,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SealedLetterStatus {
    Sealed,
    Opened { opened_at_epoch: Epoch },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedLetter {
    pub id: LetterId,
    /// Sender (the parent / grandparent / hospice patient).
    pub sender: AccountAddress,
    /// Recipient's DID, modeled here as an account address. The DID
    /// layer (out of scope) provides the verified `birth_epoch`.
    pub recipient_did: AccountAddress,
    /// Epoch corresponding to recipient's birth — supplied by the
    /// DID layer. Validators trust the DID's attestation; this crate
    /// just consumes the value.
    pub recipient_birth_epoch: Epoch,
    /// Age (in years) at which the letter unlocks. The chain's
    /// epochs-per-year conversion is in `epochs_per_year`.
    pub unlock_age_years: u32,
    /// Conversion factor: `unlock_epoch = birth_epoch +
    /// unlock_age_years * epochs_per_year`. Stored on the letter so
    /// chain-time changes don't retro-shift unlock dates.
    pub epochs_per_year: u64,
    /// Encrypted-blob commitment.
    pub vault: VaultBlob,
    /// Lifecycle.
    pub status: SealedLetterStatus,
    /// Epoch the letter was sealed (purely informational; not used
    /// for unlock math).
    pub sealed_at_epoch: Epoch,
}

impl SealedLetter {
    pub fn seal(
        id: LetterId,
        sender: AccountAddress,
        recipient_did: AccountAddress,
        recipient_birth_epoch: Epoch,
        unlock_age_years: u32,
        epochs_per_year: u64,
        vault: VaultBlob,
        sealed_at_epoch: Epoch,
    ) -> Result<Self, LetterError> {
        if unlock_age_years == 0 {
            return Err(LetterError::ZeroUnlockAge);
        }
        if epochs_per_year == 0 {
            return Err(LetterError::ZeroEpochsPerYear);
        }
        Ok(Self {
            id,
            sender,
            recipient_did,
            recipient_birth_epoch,
            unlock_age_years,
            epochs_per_year,
            vault,
            status: SealedLetterStatus::Sealed,
            sealed_at_epoch,
        })
    }

    /// Compute the absolute epoch at which the letter unlocks.
    pub fn unlock_epoch(&self) -> Epoch {
        let years = self.unlock_age_years as u64;
        self.recipient_birth_epoch
            .saturating_add(years.saturating_mul(self.epochs_per_year))
    }

    pub fn is_sealed(&self) -> bool {
        matches!(self.status, SealedLetterStatus::Sealed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::{KeyShareCommitment, VaultBlob};

    fn validator(b: u8) -> KeyShareCommitment {
        let mut a = [0u8; 32];
        a[0] = b;
        KeyShareCommitment { validator: a }
    }

    fn vault() -> VaultBlob {
        VaultBlob::new(
            [0xAB; 32],
            1024,
            2,
            vec![validator(1), validator(2), validator(3)],
        )
        .unwrap()
    }

    #[test]
    fn seal_rejects_zero_unlock_age() {
        let err = SealedLetter::seal(
            [0; 32],
            [0xAA; 32],
            [0xBB; 32],
            1000,
            0,
            365,
            vault(),
            0,
        )
        .unwrap_err();
        assert_eq!(err, LetterError::ZeroUnlockAge);
    }

    #[test]
    fn seal_rejects_zero_epochs_per_year() {
        let err = SealedLetter::seal(
            [0; 32],
            [0xAA; 32],
            [0xBB; 32],
            1000,
            18,
            0,
            vault(),
            0,
        )
        .unwrap_err();
        assert_eq!(err, LetterError::ZeroEpochsPerYear);
    }

    #[test]
    fn unlock_epoch_is_birth_plus_years_times_factor() {
        let l = SealedLetter::seal(
            [0; 32],
            [0xAA; 32],
            [0xBB; 32],
            10_000, // birth at epoch 10_000
            18,
            365,    // 365 epochs per year (1 epoch = 1 day)
            vault(),
            0,
        )
        .unwrap();
        // 10_000 + 18 * 365 = 16_570
        assert_eq!(l.unlock_epoch(), 16_570);
    }

    #[test]
    fn newly_sealed_letter_is_in_sealed_state() {
        let l = SealedLetter::seal(
            [0; 32],
            [0xAA; 32],
            [0xBB; 32],
            0,
            18,
            365,
            vault(),
            0,
        )
        .unwrap();
        assert!(l.is_sealed());
    }

    #[test]
    fn round_trip_serde() {
        let l = SealedLetter::seal(
            [0xCD; 32],
            [0xAA; 32],
            [0xBB; 32],
            5000,
            18,
            365,
            vault(),
            10,
        )
        .unwrap();
        let s = serde_json::to_string(&l).unwrap();
        let back: SealedLetter = serde_json::from_str(&s).unwrap();
        assert_eq!(l, back);
    }
}
