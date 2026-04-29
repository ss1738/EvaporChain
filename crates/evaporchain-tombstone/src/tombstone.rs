//! `Tombstone` — 32-byte memorial commitment.

use serde::{Deserialize, Serialize};

use evaporchain_types::{AccountAddress, Energy};

use crate::cause::CauseOfDeath;

const TOMBSTONE_TAG: &[u8] = b"evaporchain-tombstone";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Tombstone {
    pub commitment: [u8; 32],
}

/// Mint a `Tombstone` for the address that just evaporated.
///
/// `commitment = blake3("evaporchain-tombstone" || addr ||
/// final_balance || final_epoch || cause_discriminant)`.
///
/// Domain-separated so the commitment can never collide with any
/// other on-chain hash. Two evaporated accounts with identical
/// (addr, final_balance, final_epoch, cause) tuples produce the
/// same tombstone — but that tuple is unique per account by
/// construction (an address can only evaporate once).
pub fn mint(
    addr: AccountAddress,
    final_balance: Energy,
    final_epoch: u64,
    cause: CauseOfDeath,
) -> Tombstone {
    let mut h = blake3::Hasher::new();
    h.update(TOMBSTONE_TAG);
    h.update(&addr);
    h.update(&final_balance.to_le_bytes());
    h.update(&final_epoch.to_le_bytes());
    h.update(&cause.discriminant().to_le_bytes());
    Tombstone {
        commitment: *h.finalize().as_bytes(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(b: u8) -> AccountAddress {
        [b; 32]
    }

    #[test]
    fn mint_is_deterministic() {
        let t1 = mint(addr(1), 0, 100, CauseOfDeath::Evaporated);
        let t2 = mint(addr(1), 0, 100, CauseOfDeath::Evaporated);
        assert_eq!(t1, t2);
    }

    #[test]
    fn different_address_different_commitment() {
        let t1 = mint(addr(1), 0, 100, CauseOfDeath::Evaporated);
        let t2 = mint(addr(2), 0, 100, CauseOfDeath::Evaporated);
        assert_ne!(t1, t2);
    }

    #[test]
    fn different_cause_different_commitment() {
        let t1 = mint(addr(1), 0, 100, CauseOfDeath::Evaporated);
        let t2 = mint(addr(1), 0, 100, CauseOfDeath::SlashedToZero);
        assert_ne!(t1, t2);
    }

    #[test]
    fn different_epoch_different_commitment() {
        let t1 = mint(addr(1), 0, 100, CauseOfDeath::Evaporated);
        let t2 = mint(addr(1), 0, 200, CauseOfDeath::Evaporated);
        assert_ne!(t1, t2);
    }

    #[test]
    fn final_balance_in_commitment() {
        // Some causes (e.g. ForgottenViaDecayProof) may evaporate at a
        // non-zero ledger balance; the tombstone records that residual.
        let t1 = mint(addr(1), 50, 100, CauseOfDeath::ForgottenViaDecayProof);
        let t2 = mint(addr(1), 100, 100, CauseOfDeath::ForgottenViaDecayProof);
        assert_ne!(t1, t2);
    }
}
