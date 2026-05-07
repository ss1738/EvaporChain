//! MERA State Commitment integration — wires `evaporchain-mera` into the
//! execution layer.
//!
//! After each block execution the executor collects all account energy values
//! from the `StateDB`, builds a MERA tree over them, and returns the 32-byte
//! `root_hash` as the block's `mera_commitment`.
//!
//! # Why MERA instead of a flat Merkle tree
//!
//! A flat Merkle tree treats every account as independent.  MERA coarse-grains
//! the energy distribution through a λ-parameterised tensor network: layer ℓ
//! captures correlations at scale 2^ℓ accounts with a half-life of τ₀·2^ℓ.
//! The root hash is a *renormalization-group-invariant* commitment — small
//! perturbations to individual balances produce predictable changes to the
//! upper layers, which the Light-Cone Ledger can exploit for fraud proofs.
//!
//! # Flow
//!
//! `compute_mera_commitment(db, lambda_half_life)`:
//!   1. Collect all account balances via `db.all_account_addresses()` →
//!      `db.get_account(addr).map(|a| a.balance)`.
//!   2. Call `evaporchain_mera::commit(energies, lambda_half_life, base_half_life)`.
//!   3. Return `commitment.root_hash`.
//!
//! Best-effort: if the state is empty, returns the all-zeros hash.

use evaporchain_energy_kernel::DEFAULT_LAMBDA;
use evaporchain_mera::commit;
use evaporchain_state::db::StateDB;
use tracing::debug;

/// Layer-0 half-life (τ₀) for the MERA tree.  Per the MERA gate result
/// (GATE_RESULT.md), τ₀ = 1 epoch is the minimal physical scale.
pub const MERA_BASE_HALF_LIFE: u64 = 1;

/// Compute the MERA state commitment for all accounts currently in `db`.
///
/// Returns the 32-byte `root_hash` from `MeraCommitment`.  Returns
/// `[0u8; 32]` if there are no accounts (genesis edge case).
pub fn compute_mera_commitment(db: &dyn StateDB) -> [u8; 32] {
    let addrs = db.all_account_addresses();
    if addrs.is_empty() {
        return [0u8; 32];
    }

    let energies: Vec<u64> = addrs
        .iter()
        .filter_map(|addr| db.get_account(addr))
        .map(|acct| acct.balance)
        .collect();

    let lambda_half_life = DEFAULT_LAMBDA.epochs().max(1);
    let (commitment, _tree) = commit(&energies, lambda_half_life, MERA_BASE_HALF_LIFE);

    debug!(
        accounts = energies.len(),
        root_hash = hex::encode(commitment.root_hash),
        lambda_half_life,
        "MERA state commitment computed"
    );

    commitment.root_hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_state::db::InMemoryStateDB;
    use evaporchain_types::{Account, AccountAddress};

    fn addr(b: u8) -> AccountAddress {
        [b; 32]
    }

    fn make_db(balances: &[(u8, u64)]) -> InMemoryStateDB {
        let mut db = InMemoryStateDB::new();
        for &(b, bal) in balances {
            db.put_account(Account {
                address: addr(b),
                balance: bal,
                nonce: 0,
                storage_deposit: 0,
                storage_bytes: 0,
                last_touched_epoch: 0,
                vesting: None,
            });
        }
        db
    }

    #[test]
    fn empty_db_returns_zero_hash() {
        let db = InMemoryStateDB::new();
        assert_eq!(compute_mera_commitment(&db), [0u8; 32]);
    }

    #[test]
    fn non_empty_db_returns_nonzero_hash() {
        let db = make_db(&[(1, 10_000), (2, 20_000)]);
        let h = compute_mera_commitment(&db);
        assert_ne!(h, [0u8; 32]);
    }

    #[test]
    fn same_state_same_commitment() {
        let db = make_db(&[(1, 100), (2, 200), (3, 300)]);
        let h1 = compute_mera_commitment(&db);
        let h2 = compute_mera_commitment(&db);
        assert_eq!(h1, h2);
    }

    #[test]
    fn different_balances_different_commitment() {
        let db1 = make_db(&[(1, 100), (2, 200)]);
        let db2 = make_db(&[(1, 100), (2, 999)]);
        let h1 = compute_mera_commitment(&db1);
        let h2 = compute_mera_commitment(&db2);
        assert_ne!(h1, h2);
    }
}
