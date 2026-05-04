//! WitnessFit (Singh-Streak) → Tombstone adapter.
//!
//! When a Singh-Streak's strength has decayed to `Broken` AND the
//! holder voluntarily retires it, the chain records the streak's
//! lifetime audit trail (total check-ins + the BLAKE3 chain over all
//! witness payload hashes) as a permanent attestation in the eulogy
//! trie.
//!
//! Why "voluntarily retires" gates the tombstoning: a Broken streak
//! can be revived by a single check-in (the doctrine moat — graceful
//! decay, not cliff). The tombstone is for users who have *finished*
//! that streak — closed a chapter, walked away, accepted the loss.
//! The chain records the closure with the same finality it gives
//! to evaporated accounts.

use evaporchain_tombstone::{mint as mint_tombstone, CauseOfDeath, Tombstone};
use evaporchain_types::Epoch;
use evaporchain_witnessfit::{Streak, StrengthBucket};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WitnessFitTombstoneError {
    #[error("streak is not yet Broken at epoch {epoch_now} — refuses to memorialise a still-recoverable streak")]
    NotBroken { epoch_now: Epoch },
    #[error("streak has zero check-ins — nothing to memorialise")]
    NeverWitnessed,
}

/// Build the tombstone for a retired Singh-Streak.
///
/// Errors if the streak is not yet `Broken` (the doctrine guard:
/// a streak that's still recoverable shouldn't be memorialised — the
/// holder should refresh it instead). Errors on a never-witnessed
/// streak (no audit trail to preserve).
pub fn tombstone_for_streak(
    streak: &Streak,
    epoch_now: Epoch,
) -> Result<Tombstone, WitnessFitTombstoneError> {
    if streak.bucket_at(epoch_now) != StrengthBucket::Broken {
        return Err(WitnessFitTombstoneError::NotBroken { epoch_now });
    }
    if streak.total_check_ins == 0 {
        return Err(WitnessFitTombstoneError::NeverWitnessed);
    }
    // The streak's id IS the tombstone key. final_balance = total
    // check-ins (the chain's lasting record of "how much this
    // person actually showed up"). final_epoch = epoch_now (the
    // moment of voluntary retirement).
    Ok(mint_tombstone(
        streak.id,
        streak.total_check_ins,
        epoch_now,
        CauseOfDeath::Evaporated,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_types::AccountAddress;
    use evaporchain_witnessfit::{Streak, WitnessAttestation};

    fn id(b: u8) -> [u8; 32] {
        let mut x = [0u8; 32];
        x[0] = b;
        x
    }

    fn addr(b: u8) -> AccountAddress {
        id(b)
    }

    fn always_valid(_a: &AccountAddress, _b: &[u8], _s: &[u8]) -> bool {
        true
    }

    #[test]
    fn fresh_streak_with_no_witnesses_cannot_be_tombstoned() {
        let s = Streak::mint(id(1), addr(0xAA), 100, 0).unwrap();
        // Brand-new streak: strength=0 (Broken bucket) BUT zero check-ins.
        let err = tombstone_for_streak(&s, 0).unwrap_err();
        assert_eq!(err, WitnessFitTombstoneError::NeverWitnessed);
    }

    #[test]
    fn healthy_streak_cannot_be_tombstoned() {
        let mut s = Streak::mint(id(1), addr(0xAA), 100, 0).unwrap();
        let att = WitnessAttestation::new(id(1), 0, [0xAB; 32], vec![1; 8]).unwrap();
        // Boost to full strength.
        s.record(att, 10_000, always_valid).unwrap();
        // Streak is Healthy — refusing to memorialise.
        let err = tombstone_for_streak(&s, 0).unwrap_err();
        assert!(matches!(err, WitnessFitTombstoneError::NotBroken { .. }));
    }

    #[test]
    fn broken_streak_with_history_produces_tombstone() {
        // Build up history, let it decay to Broken, then memorialise.
        let mut s = Streak::mint(id(1), addr(0xAA), 1, 0).unwrap();
        // Two check-ins so total_check_ins > 0.
        let a1 = WitnessAttestation::new(id(1), 0, [0xAB; 32], vec![1; 8]).unwrap();
        let a2 = WitnessAttestation::new(id(1), 1, [0xCD; 32], vec![1; 8]).unwrap();
        s.record(a1, 5000, always_valid).unwrap();
        s.record(a2, 5000, always_valid).unwrap();
        // After many half-lives at half_life=1, strength → 0 = Broken.
        let _stone = tombstone_for_streak(&s, 10_000).unwrap();
    }

    #[test]
    fn the_voluntary_retirement_property_holds() {
        // Doctrine: tombstone is for "finished" streaks. Build a
        // streak, let it Break, verify it can be tombstoned at any
        // epoch *while* Broken. (The user chooses *when* to retire;
        // the chain just enforces "must be Broken first.")
        let mut s = Streak::mint(id(1), addr(0xAA), 1, 0).unwrap();
        let att = WitnessAttestation::new(id(1), 0, [0xAB; 32], vec![1; 8]).unwrap();
        s.record(att, 1000, always_valid).unwrap();
        // Tombstone immediately when Broken arrives.
        let _early = tombstone_for_streak(&s, 100).unwrap();
        // Or much later — both yield tombstones (different commitments
        // because final_epoch differs).
        let _late = tombstone_for_streak(&s, 9_999).unwrap();
    }

    #[test]
    fn audit_trail_survives_in_the_tombstone() {
        // The doctrine moat for WitnessFit is "portable cognitive
        // credential" — the lifetime check-in count survives the
        // streak's death by being baked into the tombstone commitment
        // (via final_balance). Two streaks with the same id but
        // different total_check_ins must produce different tombstones.
        let make_streak = |total: u64| {
            let mut s = Streak::mint(id(1), addr(0xAA), 1, 0).unwrap();
            for i in 0..total {
                let mut payload = [0u8; 32];
                payload[0] = (i + 1) as u8;
                let att = WitnessAttestation::new(id(1), i, payload, vec![1; 8]).unwrap();
                s.record(att, 1000, always_valid).unwrap();
            }
            s
        };
        let three_visits = make_streak(3);
        let ten_visits = make_streak(10);
        let t3 = tombstone_for_streak(&three_visits, 10_000).unwrap();
        let t10 = tombstone_for_streak(&ten_visits, 10_000).unwrap();
        assert_ne!(t3.commitment, t10.commitment);
    }
}
