//! Mayfly → Tombstone adapter.
//!
//! When a Provably-Mortal NFT (Gallery Mayfly) reaches its certified
//! death epoch, the chain commits its passing to the eulogy trie.
//!
//! The address used as the tombstone key is derived deterministically
//! from the Mayfly's `id` (a 32-byte hash) so the tombstone is
//! reproducible from the Mayfly alone. We do NOT use the owner's
//! address — multiple Mayflies can belong to the same owner and they
//! must each get their own memorial.

use evaporchain_gallery_forgets::MayflyToken;
use evaporchain_tombstone::{mint as mint_tombstone, CauseOfDeath, Tombstone};
use evaporchain_types::Epoch;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MayflyTombstoneError {
    #[error("mayfly is not yet dead at epoch {epoch_now} (certificate: {projected})")]
    NotYetDead { epoch_now: Epoch, projected: Epoch },
    #[error("mayfly certificate failed verification — refusing to memorialise")]
    InvalidCertificate,
}

/// Build the tombstone for a Mayfly that has died.
///
/// Returns `Err(NotYetDead)` if the Mayfly is still alive at
/// `epoch_now`. Returns `Err(InvalidCertificate)` if the Mayfly's
/// `DeathCertificate` does not verify against its parameters
/// (defence against forged tokens being memorialised).
pub fn tombstone_for_mayfly(
    mayfly: &MayflyToken,
    epoch_now: Epoch,
) -> Result<Tombstone, MayflyTombstoneError> {
    if !mayfly.verify_certificate() {
        return Err(MayflyTombstoneError::InvalidCertificate);
    }
    if !mayfly.is_dead(epoch_now) {
        return Err(MayflyTombstoneError::NotYetDead {
            epoch_now,
            projected: mayfly.certificate.projected_death_epoch,
        });
    }
    // The Mayfly's id IS the address-shaped 32-byte handle for the
    // tombstone. final_balance is the score at death (== 0 by
    // construction since Mayflies have floor=0). final_epoch is
    // epoch_now (the actual death observation, not the projection).
    Ok(mint_tombstone(
        mayfly.id,
        0, // Mayflies die at zero — that's the whole point.
        epoch_now,
        CauseOfDeath::Evaporated,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_gallery_forgets::MayflyToken;
    use evaporchain_types::AccountAddress;

    fn id(b: u8) -> [u8; 32] {
        let mut x = [0u8; 32];
        x[0] = b;
        x
    }

    fn addr(b: u8) -> AccountAddress {
        id(b)
    }

    #[test]
    fn fresh_mayfly_cannot_be_memorialised() {
        let m = MayflyToken::mint(id(1), addr(0xAA), 1000, 100, 0).unwrap();
        let err = tombstone_for_mayfly(&m, 0).unwrap_err();
        assert!(matches!(err, MayflyTombstoneError::NotYetDead { .. }));
    }

    #[test]
    fn dead_mayfly_produces_tombstone() {
        // Tiny initial + small half-life ⇒ token dies in finite time.
        let m = MayflyToken::mint(id(1), addr(0xAA), 4, 1, 0).unwrap();
        // After many half-lives the Mayfly is dead.
        let _t = tombstone_for_mayfly(&m, 1_000).unwrap();
        // Tombstone is a 32-byte commitment; we don't pry into its
        // structure here — just verify it produced one.
    }

    #[test]
    fn forged_certificate_rejected() {
        // Tamper the certificate's commitment so verify_certificate fails.
        let mut m = MayflyToken::mint(id(1), addr(0xAA), 4, 1, 0).unwrap();
        m.certificate.commitment[0] ^= 0xFF;
        let err = tombstone_for_mayfly(&m, 1_000).unwrap_err();
        assert_eq!(err, MayflyTombstoneError::InvalidCertificate);
    }

    #[test]
    fn the_lifecycle_completes_end_to_end() {
        // Doctrine claim: a Provably-Mortal NFT mints a death
        // certificate at birth, dies provably, and is memorialised
        // forever. Walk the whole arc.
        let m = MayflyToken::mint(id(7), addr(0xAA), 4, 1, 0).unwrap();
        // 1. At mint, certificate is verifiable.
        assert!(m.verify_certificate());
        // 2. At certified death epoch, token is dead.
        let promised = m.certificate.projected_death_epoch;
        assert!(m.is_dead(promised));
        // 3. The chain mints a tombstone — small death recorded.
        let _ = tombstone_for_mayfly(&m, promised).unwrap();
    }

    #[test]
    fn two_mayflies_get_distinct_tombstones() {
        // Different Mayfly ids → different tombstone keys, even if
        // owned by the same wallet.
        let a = MayflyToken::mint(id(1), addr(0xAA), 4, 1, 0).unwrap();
        let b = MayflyToken::mint(id(2), addr(0xAA), 4, 1, 0).unwrap();
        let ta = tombstone_for_mayfly(&a, 1_000).unwrap();
        let tb = tombstone_for_mayfly(&b, 1_000).unwrap();
        assert_ne!(ta.commitment, tb.commitment);
    }
}
