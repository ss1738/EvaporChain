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
