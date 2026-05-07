pub mod db;
pub mod decay_curves;
pub mod evaporation;
pub mod ghost_bridge;
pub(crate) mod legacy;
pub mod refresh;
pub mod rocksdb_backend;
pub mod snapshot;
pub mod sync;
pub mod wal;

pub use db::{InMemoryStateDB, StateDB};
pub use evaporation::{EvaporationEngine, EvaporationResult};
pub use evaporchain_crypto::TrieHealth;
pub use refresh::{RefreshEngine, RefreshError};
pub use rocksdb_backend::RocksDBStateDB;
pub use snapshot::{
    ContractEntry, SnapshotBellReading, SnapshotFile, SnapshotMetadata, SnapshotValidator,
    ValidatorSetSnapshot, SNAPSHOT_COMPRESSION_LEVEL, SNAPSHOT_FILE_VERSION, SNAPSHOT_MAGIC,
};
pub use wal::{WalEntry, WalMutation, WriteAheadLog};

#[cfg(test)]
mod press_claim_tests {
    use super::*;
    use evaporchain_types::{Account, AccountAddress};

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "evaporchain-state's `compute_state_root` is a
    /// pure function of the active object/account set: same set →
    /// same root; flipping any account balance produces a different
    /// root; double-spending a nullifier is rejected; nullifier
    /// uniqueness is enforced by the spend gate (not by the caller)."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        fn addr(b: u8) -> AccountAddress {
            [b; 32]
        }

        let mut db1 = InMemoryStateDB::new();
        db1.put_account(Account {
            address: addr(1),
            balance: 100,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: None,
        });
        db1.put_account(Account {
            address: addr(2),
            balance: 200,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: None,
        });
        let r1 = db1.compute_state_root();

        // Reproducibility: same DB → same root.
        let r1_again = db1.compute_state_root();
        assert_eq!(r1, r1_again);

        // Equivalence: separately-built DB with the same entries →
        // same root (commutativity of inserts).
        let mut db2 = InMemoryStateDB::new();
        db2.put_account(Account {
            address: addr(2),
            balance: 200,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: None,
        });
        db2.put_account(Account {
            address: addr(1),
            balance: 100,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: None,
        });
        let r2 = db2.compute_state_root();
        assert_eq!(r1, r2);

        // Tamper: flip a balance → root must change.
        db2.put_account(Account {
            address: addr(1),
            balance: 999,
            nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            vesting: None,
        });
        let r2_tampered = db2.compute_state_root();
        assert_ne!(r1, r2_tampered);

        // Nullifier double-spend rejected.
        let n = [0xAA; 32];
        let mut nf = InMemoryStateDB::new();
        assert!(nf.spend_nullifier(&n), "first spend must succeed");
        assert!(!nf.spend_nullifier(&n), "double-spend must reject");
        assert!(nf.is_nullifier_spent(&n));
    }
}
