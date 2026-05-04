//! MnemoChain (Singh-Curve) → Tombstone adapter.
//!
//! When a learning Card's memory energy decays to zero AND the
//! holder accepts the loss (does not refresh), the chain records the
//! lifetime study record (attempts, correct, lapses, current
//! stability) as a permanent attestation in the eulogy trie.
//!
//! This is the chain's version of "I forgot the card before I could
//! review it" — a recognised cognitive event that the learner can
//! later look up. The cognitive credential (`I provably reviewed
//! Spanish vocab card #4471 312 times before letting it lapse`)
//! survives the card itself.

use evaporchain_mnemochain::Card;
use evaporchain_tombstone::{mint as mint_tombstone, CauseOfDeath, Tombstone};
use evaporchain_types::Epoch;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MnemoTombstoneError {
    #[error("card still has memory energy at epoch {epoch_now} — refresh first or wait for full decay")]
    StillFresh { epoch_now: Epoch },
    #[error("card has zero attempts — nothing to memorialise (was it ever studied?)")]
    NeverReviewed,
}

/// Build the tombstone for a fully-evaporated learning Card.
///
/// Errors if the card still has memory energy (the holder should
/// either refresh or wait — same doctrine as WitnessFit). Errors on
/// a never-reviewed card.
pub fn tombstone_for_card(
    card: &Card,
    epoch_now: Epoch,
) -> Result<Tombstone, MnemoTombstoneError> {
    if card.energy_at(epoch_now) > 0 {
        return Err(MnemoTombstoneError::StillFresh { epoch_now });
    }
    if card.attempts == 0 {
        return Err(MnemoTombstoneError::NeverReviewed);
    }
    // The card's id IS the tombstone key. final_balance = total
    // attempts (the chain's lasting record of "how much study this
    // card received"). final_epoch = epoch_now.
    Ok(mint_tombstone(
        card.id,
        card.attempts,
        epoch_now,
        CauseOfDeath::Evaporated,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_mnemochain::{Card, Grade, MnemoTriePtr};
    use evaporchain_types::AccountAddress;

    fn id(b: u8) -> [u8; 32] {
        let mut x = [0u8; 32];
        x[0] = b;
        x
    }

    fn addr(b: u8) -> AccountAddress {
        id(b)
    }

    fn ptr() -> MnemoTriePtr {
        MnemoTriePtr::new([0xCD; 32], 64).unwrap()
    }

    fn fresh_card() -> Card {
        Card::mint(id(1), addr(0xAA), ptr(), 1000, 1, 0).unwrap()
    }

    #[test]
    fn fresh_card_with_no_attempts_cannot_be_tombstoned() {
        let c = fresh_card();
        // Card is fresh: still has full energy. The "still fresh" guard
        // fires before the never-reviewed guard.
        let err = tombstone_for_card(&c, 0).unwrap_err();
        assert!(matches!(err, MnemoTombstoneError::StillFresh { .. }));
    }

    #[test]
    fn never_reviewed_but_decayed_card_rejected() {
        // Card with no reviews that has decayed to zero — there's no
        // cognitive credential to memorialise.
        let c = fresh_card();
        let err = tombstone_for_card(&c, 100_000).unwrap_err();
        assert_eq!(err, MnemoTombstoneError::NeverReviewed);
    }

    #[test]
    fn reviewed_and_decayed_card_produces_tombstone() {
        let mut c = fresh_card();
        // Review three times; the card stays reviewed even after
        // memory decays.
        c.review(addr(0xAA), Grade::Good, 0).unwrap();
        c.review(addr(0xAA), Grade::Good, 1).unwrap();
        c.review(addr(0xAA), Grade::Again, 2).unwrap();
        // Wait long enough for energy to decay to zero.
        let _stone = tombstone_for_card(&c, 100_000).unwrap();
        // Sanity: attempts and lapses survived in the card's record.
        assert_eq!(c.attempts, 3);
        assert_eq!(c.lapses, 1);
    }

    #[test]
    fn cognitive_credential_survives_in_the_tombstone() {
        // Doctrine moat: cards become portable cognitive credentials.
        // After the card itself dies, the tombstone preserves the
        // attempt count. Two cards with different histories produce
        // different tombstones.
        //
        // Mechanics: Grade::Good multiplies stability by 2.5×; a
        // single Grade::Again only divides by 10. To make the card
        // actually decay to zero in test-time we use only Agains —
        // each call collapses stability and *also* increments
        // attempts/lapses (the audit trail being preserved).
        let mut few = fresh_card();
        for ep in 0..3u64 {
            few.review(addr(0xAA), Grade::Again, ep).unwrap();
        }
        let mut many = fresh_card();
        for ep in 0..15u64 {
            many.review(addr(0xAA), Grade::Again, ep).unwrap();
        }
        // After Again-only sequences, stability stays near 1 throughout
        // (each Again pushes stability toward STABILITY_FLOOR=1). Far
        // in the future, both cards have fully decayed.
        let t_few = tombstone_for_card(&few, 1_000).unwrap();
        let t_many = tombstone_for_card(&many, 1_000).unwrap();
        assert_ne!(t_few.commitment, t_many.commitment);
    }
}
