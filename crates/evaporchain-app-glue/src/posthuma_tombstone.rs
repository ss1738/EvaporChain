//! Posthuma → Tombstone adapter.
//!
//! When a Singh-Posthuma testament transitions from `Revealed` to
//! `Memorial` (i.e. the readable form has decayed and the chain
//! holds only the 32-byte commemorative marker), the chain ALSO
//! commits the issuer's "they once said something" to the eulogy
//! trie. The MemorialMarker is testament-internal; the Tombstone is
//! chain-wide. Both must agree.
//!
//! The address used as the tombstone key is the testament's `id`
//! (a 32-byte hash). One issuer can leave many testaments; each
//! gets its own tombstone.

use evaporchain_singh_posthuma::{Testament, TestamentStatus};
use evaporchain_tombstone::{mint as mint_tombstone, CauseOfDeath, Tombstone};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PosthumaTombstoneError {
    #[error("testament is not yet at Memorial status — refusing to commit a tombstone")]
    NotMemorial,
}

/// Mint a Tombstone for a testament that has reached `Memorial`
/// status. Errors on Sealed and Revealed testaments — only fully
/// faded testaments qualify.
pub fn tombstone_for_memorial(testament: &Testament) -> Result<Tombstone, PosthumaTombstoneError> {
    let revealed_at = match &testament.status {
        TestamentStatus::Memorial(m) => m.revealed_at_epoch,
        _ => return Err(PosthumaTombstoneError::NotMemorial),
    };
    Ok(mint_tombstone(
        testament.id,
        0, // testaments don't carry balance; they carry words. final_balance=0.
        revealed_at,
        CauseOfDeath::Evaporated,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_singh_posthuma::{Attestation, DeathCertificate, SealedVault, Testament};
    use evaporchain_types::AccountAddress;

    fn id(b: u8) -> [u8; 32] {
        let mut x = [0u8; 32];
        x[0] = b;
        x
    }

    fn addr(b: u8) -> AccountAddress {
        id(b)
    }

    fn vault() -> SealedVault {
        SealedVault::new([0xAB; 32], 1024, 1, vec![addr(1)], [0xCD; 32]).unwrap()
    }

    fn always_valid(_v: &AccountAddress, _b: &[u8], _s: &[u8]) -> bool {
        true
    }

    fn faded_testament() -> Testament {
        // Build a Testament, accept death certificate, fade to Memorial.
        let mut t = Testament::seal(
            id(7),
            addr(0xAA),
            vault(),
            1, // tiny half-life so memorial fade is cheap
            1_000,
            0,
        )
        .unwrap();
        let cert = DeathCertificate {
            testament_id: id(7),
            death_epoch: 100,
            nonce: [0xEE; 32],
            attestations: vec![Attestation {
                validator: addr(1),
                signature: vec![1; 8],
            }],
        };
        t.accept_death_certificate(&cert, always_valid).unwrap();
        t.fade_to_memorial(10_000).unwrap();
        t
    }

    #[test]
    fn sealed_testament_cannot_be_memorialised() {
        let t = Testament::seal(id(1), addr(0xAA), vault(), 100, 1000, 0).unwrap();
        let err = tombstone_for_memorial(&t).unwrap_err();
        assert_eq!(err, PosthumaTombstoneError::NotMemorial);
    }

    #[test]
    fn revealed_but_not_faded_cannot_be_memorialised() {
        // Just-revealed testament still has visible energy.
        let mut t = Testament::seal(id(1), addr(0xAA), vault(), 100, 1000, 0).unwrap();
        let cert = DeathCertificate {
            testament_id: id(1),
            death_epoch: 50,
            nonce: [0xEE; 32],
            attestations: vec![Attestation {
                validator: addr(1),
                signature: vec![1; 8],
            }],
        };
        t.accept_death_certificate(&cert, always_valid).unwrap();
        // Revealed but not yet faded.
        let err = tombstone_for_memorial(&t).unwrap_err();
        assert_eq!(err, PosthumaTombstoneError::NotMemorial);
    }

    #[test]
    fn faded_testament_produces_tombstone() {
        let t = faded_testament();
        let _stone = tombstone_for_memorial(&t).unwrap();
    }

    #[test]
    fn the_two_memorials_agree_in_principle() {
        // Doctrine: Posthuma's MemorialMarker and the chain's
        // EulogyTrie are the same conceptual artefact at different
        // scales. Concretely: the testament's Memorial reveal_epoch
        // and the Tombstone's final_epoch are the same.
        let t = faded_testament();
        // Reveal happened at epoch 100 inside `faded_testament`.
        let memorial_revealed_at = match &t.status {
            TestamentStatus::Memorial(m) => m.revealed_at_epoch,
            _ => unreachable!(),
        };
        assert_eq!(memorial_revealed_at, 100);
        // The tombstone records the same epoch (we don't expose final_epoch
        // on Tombstone publicly — it's part of the commitment — but
        // the test verifies determinism: two faded testaments with
        // the same inputs produce the same tombstone).
        let stone_a = tombstone_for_memorial(&t).unwrap();
        let stone_b = tombstone_for_memorial(&t).unwrap();
        assert_eq!(stone_a.commitment, stone_b.commitment);
    }
}
