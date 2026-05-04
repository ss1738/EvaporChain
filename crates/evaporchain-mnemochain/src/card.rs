//! `Card` — the on-chain flashcard.
//!
//! Holds the FSRS state (`stability`, `last_reviewed_at_epoch`,
//! attempts/lapses/correct counters) plus pointers to off-chain
//! content. Reviews mutate stability via the Singh Curve. Memory
//! energy at any future epoch follows the chain's standard half-life
//! rule with `half_life = stability`.
//!
//! "Due for review" is a pure-function predicate over `(stability,
//! last_reviewed_at, epoch_now)` — no scheduler service needed.

use evaporchain_types::{energy_at_epoch, AccountAddress, Energy, Epoch, HalfLife};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::curve::{update_stability, Grade, SinghCurveError, STABILITY_FLOOR};
use crate::trie::MnemoTriePtr;

/// 32-byte opaque card handle.
pub type CardId = [u8; 32];

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CardError {
    #[error("initial stability must be ≥ floor ({STABILITY_FLOOR})")]
    StabilityBelowFloor,
    #[error("review caller {caller:?} is not the card owner {owner:?}")]
    NotOwner {
        caller: AccountAddress,
        owner: AccountAddress,
    },
    #[error("review epoch {epoch} is before last_reviewed_at {last_reviewed_at} — clock backward")]
    ReviewBackwardsInTime {
        epoch: Epoch,
        last_reviewed_at: Epoch,
    },
    #[error("singh curve: {0}")]
    Curve(#[from] SinghCurveError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewOutcome {
    pub grade: Grade,
    pub new_stability: HalfLife,
    pub reviewed_at_epoch: Epoch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Card {
    pub id: CardId,
    pub owner: AccountAddress,
    pub content: MnemoTriePtr,
    /// Memory energy initial budget. The half-life used to decay it
    /// is `stability`; both update at review time.
    pub initial_energy: Energy,
    /// Current FSRS stability — also the half-life used by
    /// `energy_at` and `is_due`.
    pub stability: HalfLife,
    /// Epoch the card was last reviewed (or minted).
    pub last_reviewed_at_epoch: Epoch,
    /// Total review attempts.
    pub attempts: u64,
    /// Total `Again` reviews — the lapse counter.
    pub lapses: u64,
    /// Total non-`Again` reviews.
    pub correct: u64,
}

impl Card {
    /// Mint a new card. Initial stability is the doctrine floor; first
    /// review will grow it via the Singh curve.
    pub fn mint(
        id: CardId,
        owner: AccountAddress,
        content: MnemoTriePtr,
        initial_energy: Energy,
        initial_stability: HalfLife,
        minted_at_epoch: Epoch,
    ) -> Result<Self, CardError> {
        if initial_stability < STABILITY_FLOOR {
            return Err(CardError::StabilityBelowFloor);
        }
        Ok(Self {
            id,
            owner,
            content,
            initial_energy,
            stability: initial_stability,
            last_reviewed_at_epoch: minted_at_epoch,
            attempts: 0,
            lapses: 0,
            correct: 0,
        })
    }

    /// Energy at `epoch_now` per the standard half-life rule. Cards
    /// surface for review when their energy decays past a threshold;
    /// callers pick the threshold (typically `initial_energy / 2`,
    /// i.e. one half-life elapsed).
    pub fn energy_at(&self, epoch_now: Epoch) -> Energy {
        let elapsed = epoch_now.saturating_sub(self.last_reviewed_at_epoch);
        energy_at_epoch(self.initial_energy, self.stability, elapsed)
    }

    /// Predicate: is this card due for review at `epoch_now`?
    /// Definition: `epoch_now - last_reviewed_at ≥ stability` (one
    /// half-life elapsed). Pure function; validators agree.
    pub fn is_due(&self, epoch_now: Epoch) -> bool {
        epoch_now.saturating_sub(self.last_reviewed_at_epoch) >= self.stability
    }

    /// Apply a review. Caller must be the owner. Updates stability
    /// per the Singh curve, increments counters, and re-anchors
    /// `last_reviewed_at_epoch`.
    pub fn review(
        &mut self,
        caller: AccountAddress,
        grade: Grade,
        epoch_now: Epoch,
    ) -> Result<ReviewOutcome, CardError> {
        if caller != self.owner {
            return Err(CardError::NotOwner {
                caller,
                owner: self.owner,
            });
        }
        if epoch_now < self.last_reviewed_at_epoch {
            return Err(CardError::ReviewBackwardsInTime {
                epoch: epoch_now,
                last_reviewed_at: self.last_reviewed_at_epoch,
            });
        }
        let new_s = update_stability(self.stability, grade)?;
        self.stability = new_s;
        self.last_reviewed_at_epoch = epoch_now;
        self.attempts += 1;
        match grade {
            Grade::Again => self.lapses += 1,
            _ => self.correct += 1,
        }
        Ok(ReviewOutcome {
            grade,
            new_stability: new_s,
            reviewed_at_epoch: epoch_now,
        })
    }
}

/// Portable cognitive credential — the doctrine's headline moat.
/// "I have provably reviewed Spanish vocab card #4471 across 312
/// sessions over 4 years" — a third party (university / employer)
/// reads this and verifies the chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialAttestation {
    pub card_id: CardId,
    pub owner: AccountAddress,
    pub attempts: u64,
    pub correct: u64,
    pub lapses: u64,
    pub current_stability: HalfLife,
    pub last_reviewed_at_epoch: Epoch,
}

impl From<&Card> for CredentialAttestation {
    fn from(c: &Card) -> Self {
        Self {
            card_id: c.id,
            owner: c.owner,
            attempts: c.attempts,
            correct: c.correct,
            lapses: c.lapses,
            current_stability: c.stability,
            last_reviewed_at_epoch: c.last_reviewed_at_epoch,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(b: u8) -> CardId {
        let mut x = [0u8; 32];
        x[0] = b;
        x
    }

    fn ptr() -> MnemoTriePtr {
        MnemoTriePtr::new([0xCD; 32], 64).unwrap()
    }

    fn fresh() -> Card {
        Card::mint(id(1), [0xAA; 32], ptr(), 1_000, 10, 0).unwrap()
    }

    #[test]
    fn mint_rejects_zero_stability() {
        let err =
            Card::mint(id(1), [0xAA; 32], ptr(), 1_000, 0, 0).unwrap_err();
        assert_eq!(err, CardError::StabilityBelowFloor);
    }

    #[test]
    fn mint_records_initial_state() {
        let c = fresh();
        assert_eq!(c.attempts, 0);
        assert_eq!(c.lapses, 0);
        assert_eq!(c.correct, 0);
        assert_eq!(c.stability, 10);
    }

    #[test]
    fn review_requires_current_owner() {
        let mut c = fresh();
        let err = c.review([0xBB; 32], Grade::Good, 5).unwrap_err();
        assert!(matches!(err, CardError::NotOwner { .. }));
    }

    #[test]
    fn review_rejects_backward_time() {
        let mut c = fresh();
        c.review([0xAA; 32], Grade::Good, 100).unwrap();
        let err = c.review([0xAA; 32], Grade::Good, 50).unwrap_err();
        assert!(matches!(err, CardError::ReviewBackwardsInTime { .. }));
    }

    #[test]
    fn good_review_grows_stability() {
        let mut c = fresh();
        let outcome = c.review([0xAA; 32], Grade::Good, 5).unwrap();
        assert!(outcome.new_stability > 10);
        assert_eq!(c.attempts, 1);
        assert_eq!(c.correct, 1);
        assert_eq!(c.lapses, 0);
    }

    #[test]
    fn again_review_collapses_stability() {
        let mut c = fresh();
        // Ramp stability up first.
        c.review([0xAA; 32], Grade::Easy, 5).unwrap();
        let pre = c.stability;
        c.review([0xAA; 32], Grade::Again, 6).unwrap();
        assert!(c.stability < pre);
        assert_eq!(c.lapses, 1);
        assert_eq!(c.correct, 1);
    }

    #[test]
    fn is_due_false_immediately_after_review() {
        let mut c = fresh();
        c.review([0xAA; 32], Grade::Good, 0).unwrap();
        // Stability is now 25 (Good 2.50× of 10). Card is not due
        // until elapsed ≥ 25.
        assert!(!c.is_due(5));
        assert!(!c.is_due(24));
        assert!(c.is_due(25));
        assert!(c.is_due(1000));
    }

    #[test]
    fn portable_credential_attestation_round_trips() {
        let mut c = fresh();
        c.review([0xAA; 32], Grade::Good, 5).unwrap();
        c.review([0xAA; 32], Grade::Good, 100).unwrap();
        c.review([0xAA; 32], Grade::Again, 200).unwrap();
        let cred: CredentialAttestation = (&c).into();
        assert_eq!(cred.attempts, 3);
        assert_eq!(cred.correct, 2);
        assert_eq!(cred.lapses, 1);
        let s = serde_json::to_string(&cred).unwrap();
        let back: CredentialAttestation = serde_json::from_str(&s).unwrap();
        assert_eq!(cred, back);
    }

    #[test]
    fn the_moat_claim_lives_as_a_test() {
        // Doctrine: "I have provably reviewed card X across N sessions
        // over T years is a real attestation." Build that attestation
        // and assert it contains the load-bearing fields.
        let mut c = fresh();
        for epoch in [10, 50, 200, 800, 1800] {
            c.review([0xAA; 32], Grade::Good, epoch).unwrap();
        }
        let cred: CredentialAttestation = (&c).into();
        assert_eq!(cred.attempts, 5);
        assert_eq!(cred.correct, 5);
        // The "T years" component: the difference between
        // last_reviewed_at_epoch and the implicit minted_at_epoch (0).
        assert_eq!(cred.last_reviewed_at_epoch, 1800);
    }

    #[test]
    fn stability_growth_extends_review_interval() {
        // Doctrine claim: "correct recall re-energizes with longer
        // half-life." After successive Good grades, stability (and
        // therefore the review interval) must grow strictly.
        let mut c = fresh();
        let mut prior_s = c.stability;
        for epoch in [5, 50, 200, 800] {
            c.review([0xAA; 32], Grade::Good, epoch).unwrap();
            assert!(
                c.stability > prior_s,
                "stability must grow after Good; was {prior_s}, now {}",
                c.stability
            );
            prior_s = c.stability;
        }
    }

    #[test]
    fn round_trip_serde_card() {
        let mut c = fresh();
        c.review([0xAA; 32], Grade::Easy, 5).unwrap();
        let s = serde_json::to_string(&c).unwrap();
        let back: Card = serde_json::from_str(&s).unwrap();
        assert_eq!(c, back);
    }
}
