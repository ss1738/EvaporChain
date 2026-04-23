//! State snapshot system for EvaporChain.
//!
//! Provides serializable, verifiable state snapshots used for:
//! - **State sync** — new nodes download a snapshot instead of replaying history
//! - **Backups** — periodic checkpoints for disaster recovery
//! - **Migration** — export/import state for chain upgrades
//!
//! Snapshot format:
//!   1. Header (metadata + state root hash)
//!   2. Account records (sorted by address for deterministic hashing)
//!   3. State objects (sorted by ID)
//!   4. Ghost records (sorted by ID)
//!   5. Privacy state (nullifier set, note tree root, pool balance)
//!
//! Every snapshot is self-verifying: the state root in the header can be
//! recomputed from the contents to detect tampering.

use evaporchain_crypto::hash::blake3_hash;
use evaporchain_types::{Account, AccountAddress, GhostRecord, ObjectId, StateObject};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;
use tracing::info;

use crate::db::StateDB;

// ─────────────────────── Errors ─────────────────────────────────────────

#[derive(Debug, Error)]
pub enum SnapshotError {
    #[error("state root mismatch: expected {expected}, got {actual}")]
    StateRootMismatch { expected: String, actual: String },
    #[error("snapshot too large: {size} bytes (max {max})")]
    TooLarge { size: usize, max: usize },
    #[error("serialization error: {0}")]
    SerializationError(String),
    #[error("deserialization error: {0}")]
    DeserializationError(String),
    #[error("invalid snapshot: {0}")]
    Invalid(String),
    #[error("snapshot version mismatch: expected {expected}, got {actual}")]
    VersionMismatch { expected: u32, actual: u32 },
}

// ─────────────────────── Types ──────────────────────────────────────────

/// Current snapshot format version.
const SNAPSHOT_VERSION: u32 = 1;

/// Maximum snapshot size (1 GB).
const MAX_SNAPSHOT_SIZE: usize = 1_073_741_824;

/// Snapshot header containing metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotHeader {
    /// Format version for forward compatibility.
    pub version: u32,
    /// Block height at which the snapshot was taken.
    pub block_height: u64,
    /// Epoch at snapshot time.
    pub epoch: u64,
    /// State root hash (verifiable from contents).
    pub state_root: [u8; 32],
    /// Blake3 hash of the snapshot body (accounts + objects + ghosts + privacy).
    pub body_hash: [u8; 32],
    /// Number of accounts.
    pub account_count: u64,
    /// Number of active objects.
    pub object_count: u64,
    /// Number of ghost records.
    pub ghost_count: u64,
    /// Snapshot creation timestamp (unix seconds).
    pub created_at: u64,
    /// Total size in bytes (for bandwidth estimation).
    pub size_bytes: u64,
}

/// Privacy layer state for inclusion in snapshots.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PrivacySnapshot {
    /// Merkle note tree root.
    pub note_tree_root: [u8; 32],
    /// Set of spent nullifiers.
    pub spent_nullifiers: Vec<[u8; 32]>,
    /// Total shielded pool balance.
    pub shielded_pool_balance: u64,
    /// Note count (number of notes ever inserted).
    pub note_count: u64,
}

/// Complete state snapshot — everything needed to reconstruct the state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateSnapshot {
    /// Snapshot metadata and verification hashes.
    pub header: SnapshotHeader,
    /// All accounts, sorted by address.
    pub accounts: Vec<Account>,
    /// All active state objects, sorted by ID.
    pub objects: Vec<StateObject>,
    /// All ghost records, sorted by object ID.
    pub ghosts: Vec<GhostRecord>,
    /// Privacy layer state.
    pub privacy: PrivacySnapshot,
}

// ─────────────────────── Snapshot Builder ────────────────────────────────

/// Builds state snapshots from a StateDB.
pub struct SnapshotBuilder;

impl SnapshotBuilder {
    /// Create a snapshot from the current state database.
    pub fn create(
        db: &mut dyn StateDB,
        block_height: u64,
        epoch: u64,
    ) -> Result<StateSnapshot, SnapshotError> {
        let start = std::time::Instant::now();

        // Collect accounts (sorted for deterministic hashing)
        let mut accounts: Vec<Account> = db
            .all_account_addresses()
            .into_iter()
            .filter_map(|addr| db.get_account(&addr).cloned())
            .collect();
        accounts.sort_by(|a, b| a.address.cmp(&b.address));

        // Collect objects (sorted by ID)
        let mut objects: Vec<StateObject> = db
            .all_object_ids()
            .into_iter()
            .filter_map(|id| db.get_object(&id).cloned())
            .collect();
        objects.sort_by(|a, b| a.id.cmp(&b.id));

        // Collect ghosts (sorted by object ID)
        let mut ghosts: Vec<GhostRecord> = db
            .all_ghost_ids()
            .into_iter()
            .filter_map(|id| db.get_ghost(&id).cloned())
            .collect();
        ghosts.sort_by(|a, b| a.object_id.cmp(&b.object_id));

        // Collect privacy state
        let mut nullifiers = db.all_nullifiers();
        nullifiers.sort();
        let privacy = PrivacySnapshot {
            note_tree_root: db.get_note_tree_root(),
            spent_nullifiers: nullifiers,
            shielded_pool_balance: db.get_shielded_pool_balance(),
            note_count: db.get_note_count(),
        };

        // Compute state root
        let state_root = db.compute_state_root();

        // Compute body hash (deterministic over all content)
        let body_hash = Self::compute_body_hash(&accounts, &objects, &ghosts, &privacy);

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let header = SnapshotHeader {
            version: SNAPSHOT_VERSION,
            block_height,
            epoch,
            state_root,
            body_hash,
            account_count: accounts.len() as u64,
            object_count: objects.len() as u64,
            ghost_count: ghosts.len() as u64,
            created_at: timestamp,
            size_bytes: 0, // Updated after serialization
        };

        let mut snapshot = StateSnapshot {
            header,
            accounts,
            objects,
            ghosts,
            privacy,
        };

        // Estimate size
        let estimated_size = Self::estimate_size(&snapshot);
        snapshot.header.size_bytes = estimated_size as u64;

        if estimated_size > MAX_SNAPSHOT_SIZE {
            return Err(SnapshotError::TooLarge {
                size: estimated_size,
                max: MAX_SNAPSHOT_SIZE,
            });
        }

        info!(
            height = block_height,
            epoch,
            accounts = snapshot.accounts.len(),
            objects = snapshot.objects.len(),
            ghosts = snapshot.ghosts.len(),
            state_root = hex::encode(state_root),
            elapsed_ms = start.elapsed().as_millis() as u64,
            "State snapshot created"
        );

        Ok(snapshot)
    }

    /// Compute a deterministic hash over the snapshot body.
    fn compute_body_hash(
        accounts: &[Account],
        objects: &[StateObject],
        ghosts: &[GhostRecord],
        privacy: &PrivacySnapshot,
    ) -> [u8; 32] {
        let mut hasher_input = Vec::new();

        // Hash accounts
        for acc in accounts {
            hasher_input.extend_from_slice(&acc.address);
            hasher_input.extend_from_slice(&acc.balance.to_le_bytes());
            hasher_input.extend_from_slice(&acc.nonce.to_le_bytes());
        }

        // Hash objects
        for obj in objects {
            hasher_input.extend_from_slice(&obj.id);
            hasher_input.extend_from_slice(&obj.energy.to_le_bytes());
            hasher_input.extend_from_slice(&obj.half_life.to_le_bytes());
        }

        // Hash ghosts
        for ghost in ghosts {
            hasher_input.extend_from_slice(&ghost.object_id);
            hasher_input.extend_from_slice(&ghost.evaporated_at.to_le_bytes());
        }

        // Hash privacy state
        hasher_input.extend_from_slice(&privacy.note_tree_root);
        hasher_input.extend_from_slice(&privacy.shielded_pool_balance.to_le_bytes());
        hasher_input.extend_from_slice(&privacy.note_count.to_le_bytes());

        blake3_hash(&hasher_input)
    }

    /// Estimate the serialized size of a snapshot.
    fn estimate_size(snapshot: &StateSnapshot) -> usize {
        // Rough estimate: 100 bytes per account, 200 per object, 80 per ghost
        let base = 256; // header
        let accounts = snapshot.accounts.len() * 100;
        let objects = snapshot.objects.len() * 200;
        let ghosts = snapshot.ghosts.len() * 80;
        let nullifiers = snapshot.privacy.spent_nullifiers.len() * 32;
        base + accounts + objects + ghosts + nullifiers
    }
}

// ─────────────────────── Snapshot Applier ────────────────────────────────

/// Applies a snapshot to a StateDB, restoring state.
pub struct SnapshotApplier;

impl SnapshotApplier {
    /// Apply a snapshot to the given state database.
    ///
    /// Verifies the snapshot integrity before applying.
    pub fn apply(
        db: &mut dyn StateDB,
        snapshot: &StateSnapshot,
    ) -> Result<ApplyResult, SnapshotError> {
        // Verify version
        if snapshot.header.version != SNAPSHOT_VERSION {
            return Err(SnapshotError::VersionMismatch {
                expected: SNAPSHOT_VERSION,
                actual: snapshot.header.version,
            });
        }

        // Verify body hash
        let computed_body_hash = SnapshotBuilder::compute_body_hash(
            &snapshot.accounts,
            &snapshot.objects,
            &snapshot.ghosts,
            &snapshot.privacy,
        );
        if computed_body_hash != snapshot.header.body_hash {
            return Err(SnapshotError::StateRootMismatch {
                expected: hex::encode(snapshot.header.body_hash),
                actual: hex::encode(computed_body_hash),
            });
        }

        let start = std::time::Instant::now();

        // Apply accounts
        for acc in &snapshot.accounts {
            db.put_account(acc.clone());
        }

        // Apply objects
        for obj in &snapshot.objects {
            db.put_object(obj.clone());
        }

        // Apply ghosts
        for ghost in &snapshot.ghosts {
            db.put_ghost(ghost.clone());
        }

        // Apply privacy state
        db.put_note_tree_root(snapshot.privacy.note_tree_root);
        db.put_shielded_pool_balance(snapshot.privacy.shielded_pool_balance);
        db.put_note_count(snapshot.privacy.note_count);
        for nullifier in &snapshot.privacy.spent_nullifiers {
            db.spend_nullifier(nullifier);
        }

        // Verify state root matches after apply
        let computed_root = db.compute_state_root();
        if computed_root != snapshot.header.state_root {
            return Err(SnapshotError::StateRootMismatch {
                expected: hex::encode(snapshot.header.state_root),
                actual: hex::encode(computed_root),
            });
        }

        let elapsed = start.elapsed();

        info!(
            height = snapshot.header.block_height,
            accounts = snapshot.accounts.len(),
            objects = snapshot.objects.len(),
            ghosts = snapshot.ghosts.len(),
            elapsed_ms = elapsed.as_millis() as u64,
            "State snapshot applied"
        );

        Ok(ApplyResult {
            accounts_restored: snapshot.accounts.len(),
            objects_restored: snapshot.objects.len(),
            ghosts_restored: snapshot.ghosts.len(),
            nullifiers_restored: snapshot.privacy.spent_nullifiers.len(),
            state_root: computed_root,
            elapsed_ms: elapsed.as_millis() as u64,
        })
    }
}

/// Result of applying a snapshot.
#[derive(Debug, Clone)]
pub struct ApplyResult {
    pub accounts_restored: usize,
    pub objects_restored: usize,
    pub ghosts_restored: usize,
    pub nullifiers_restored: usize,
    pub state_root: [u8; 32],
    pub elapsed_ms: u64,
}

// ─────────────────────── Snapshot Diff ───────────────────────────────────

/// Difference between two snapshots — for incremental sync.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotDiff {
    /// Base snapshot height.
    pub base_height: u64,
    /// Target snapshot height.
    pub target_height: u64,
    /// Accounts added or modified.
    pub accounts_changed: Vec<Account>,
    /// Account addresses removed.
    pub accounts_removed: Vec<AccountAddress>,
    /// Objects added or modified.
    pub objects_changed: Vec<StateObject>,
    /// Object IDs removed.
    pub objects_removed: Vec<ObjectId>,
    /// Ghosts added.
    pub ghosts_added: Vec<GhostRecord>,
    /// Ghost IDs removed (resurrected).
    pub ghosts_removed: Vec<ObjectId>,
}

impl SnapshotDiff {
    /// Compute the diff between two snapshots.
    pub fn compute(base: &StateSnapshot, target: &StateSnapshot) -> Self {
        let base_accounts: BTreeMap<AccountAddress, &Account> =
            base.accounts.iter().map(|a| (a.address, a)).collect();
        let target_accounts: BTreeMap<AccountAddress, &Account> =
            target.accounts.iter().map(|a| (a.address, a)).collect();

        // Changed/added accounts
        let mut accounts_changed = Vec::new();
        for (addr, acc) in &target_accounts {
            match base_accounts.get(addr) {
                Some(base_acc) if acc.balance != base_acc.balance || acc.nonce != base_acc.nonce => {
                    accounts_changed.push((*acc).clone());
                }
                None => accounts_changed.push((*acc).clone()),
                _ => {}
            }
        }

        // Removed accounts
        let accounts_removed: Vec<AccountAddress> = base_accounts
            .keys()
            .filter(|addr| !target_accounts.contains_key(*addr))
            .copied()
            .collect();

        // Objects
        let base_objects: BTreeMap<ObjectId, &StateObject> =
            base.objects.iter().map(|o| (o.id, o)).collect();
        let target_objects: BTreeMap<ObjectId, &StateObject> =
            target.objects.iter().map(|o| (o.id, o)).collect();

        let mut objects_changed = Vec::new();
        for (id, obj) in &target_objects {
            match base_objects.get(id) {
                Some(base_obj) if obj.energy != base_obj.energy || obj.state != base_obj.state => {
                    objects_changed.push((*obj).clone());
                }
                None => objects_changed.push((*obj).clone()),
                _ => {}
            }
        }

        let objects_removed: Vec<ObjectId> = base_objects
            .keys()
            .filter(|id| !target_objects.contains_key(*id))
            .copied()
            .collect();

        // Ghosts
        let base_ghosts: BTreeMap<ObjectId, &GhostRecord> =
            base.ghosts.iter().map(|g| (g.object_id, g)).collect();
        let target_ghosts: BTreeMap<ObjectId, &GhostRecord> =
            target.ghosts.iter().map(|g| (g.object_id, g)).collect();

        let ghosts_added: Vec<GhostRecord> = target_ghosts
            .iter()
            .filter(|(id, _)| !base_ghosts.contains_key(*id))
            .map(|(_, g)| (*g).clone())
            .collect();

        let ghosts_removed: Vec<ObjectId> = base_ghosts
            .keys()
            .filter(|id| !target_ghosts.contains_key(*id))
            .copied()
            .collect();

        Self {
            base_height: base.header.block_height,
            target_height: target.header.block_height,
            accounts_changed,
            accounts_removed,
            objects_changed,
            objects_removed,
            ghosts_added,
            ghosts_removed,
        }
    }

    /// Total number of changes.
    pub fn total_changes(&self) -> usize {
        self.accounts_changed.len()
            + self.accounts_removed.len()
            + self.objects_changed.len()
            + self.objects_removed.len()
            + self.ghosts_added.len()
            + self.ghosts_removed.len()
    }

    /// Whether there are any differences.
    pub fn is_empty(&self) -> bool {
        self.total_changes() == 0
    }
}

// ─────────────────────── Serialization ──────────────────────────────────

/// Serialize a snapshot to bytes (bincode for compactness).
pub fn serialize_snapshot(snapshot: &StateSnapshot) -> Result<Vec<u8>, SnapshotError> {
    bincode::serialize(snapshot)
        .map_err(|e| SnapshotError::SerializationError(e.to_string()))
}

/// Deserialize a snapshot from bytes.
pub fn deserialize_snapshot(bytes: &[u8]) -> Result<StateSnapshot, SnapshotError> {
    bincode::deserialize(bytes)
        .map_err(|e| SnapshotError::DeserializationError(e.to_string()))
}

// ─────────────────────── Tests ──────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::InMemoryStateDB;
    use evaporchain_types::{ObjectState, StateObject};

    fn addr(b: u8) -> [u8; 32] {
        let mut a = [0u8; 32];
        a[0] = b;
        a
    }

    fn obj_id(b: u8) -> [u8; 32] {
        let mut id = [0u8; 32];
        id[0] = b;
        id
    }

    fn make_account(b: u8, balance: u64) -> Account {
        Account {
            address: addr(b),
            balance,
            nonce: 0,
        }
    }

    fn make_object(b: u8, energy: u64) -> StateObject {
        StateObject {
            id: obj_id(b),
            owner: addr(1),
            energy,
            half_life: 1000,
            created_at: 0,
            last_refreshed: 0,
            state: ObjectState::Active,
            grace_epoch: None,
            data: format!("object-{}", b).into_bytes(),
            decay_curve: None,
        }
    }

    fn make_ghost(b: u8, evap_epoch: u64) -> GhostRecord {
        GhostRecord {
            object_id: obj_id(b),
            owner: addr(1),
            data_hash: blake3_hash(format!("object-{}", b).as_bytes()),
            evaporated_at: evap_epoch,
            original_data: None,
            mmr_position: None,
            original_half_life: None,
        }
    }

    fn populate_db(db: &mut InMemoryStateDB) {
        db.put_account(make_account(1, 1_000_000));
        db.put_account(make_account(2, 500_000));
        db.put_account(make_account(3, 250_000));
        db.put_object(make_object(1, 100));
        db.put_object(make_object(2, 200));
        db.put_ghost(make_ghost(10, 50));
    }

    #[test]
    fn test_create_snapshot() {
        let mut db = InMemoryStateDB::new();
        populate_db(&mut db);

        let snapshot = SnapshotBuilder::create(&mut db, 100, 5).unwrap();

        assert_eq!(snapshot.header.version, SNAPSHOT_VERSION);
        assert_eq!(snapshot.header.block_height, 100);
        assert_eq!(snapshot.header.epoch, 5);
        assert_eq!(snapshot.header.account_count, 3);
        assert_eq!(snapshot.header.object_count, 2);
        assert_eq!(snapshot.header.ghost_count, 1);
        assert_ne!(snapshot.header.state_root, [0u8; 32]);
        assert_ne!(snapshot.header.body_hash, [0u8; 32]);
    }

    #[test]
    fn test_snapshot_accounts_sorted() {
        let mut db = InMemoryStateDB::new();
        // Insert in reverse order
        db.put_account(make_account(3, 300));
        db.put_account(make_account(1, 100));
        db.put_account(make_account(2, 200));

        let snapshot = SnapshotBuilder::create(&mut db, 1, 0).unwrap();

        // Should be sorted by address
        assert!(snapshot.accounts[0].address < snapshot.accounts[1].address);
        assert!(snapshot.accounts[1].address < snapshot.accounts[2].address);
    }

    #[test]
    fn test_apply_snapshot_restores_state() {
        let mut db = InMemoryStateDB::new();
        populate_db(&mut db);

        let snapshot = SnapshotBuilder::create(&mut db, 100, 5).unwrap();

        // Apply to a fresh DB
        let mut new_db = InMemoryStateDB::new();
        let result = SnapshotApplier::apply(&mut new_db, &snapshot).unwrap();

        assert_eq!(result.accounts_restored, 3);
        assert_eq!(result.objects_restored, 2);
        assert_eq!(result.ghosts_restored, 1);
        assert_eq!(result.state_root, snapshot.header.state_root);

        // Verify accounts
        assert_eq!(new_db.get_account(&addr(1)).unwrap().balance, 1_000_000);
        assert_eq!(new_db.get_account(&addr(2)).unwrap().balance, 500_000);

        // Verify objects
        assert!(new_db.get_object(&obj_id(1)).is_some());
        assert_eq!(new_db.get_object(&obj_id(1)).unwrap().energy, 100);

        // Verify ghosts
        assert!(new_db.get_ghost(&obj_id(10)).is_some());
    }

    #[test]
    fn test_snapshot_state_root_matches() {
        let mut db = InMemoryStateDB::new();
        populate_db(&mut db);

        let original_root = db.compute_state_root();
        let snapshot = SnapshotBuilder::create(&mut db, 100, 5).unwrap();

        assert_eq!(snapshot.header.state_root, original_root);

        // Apply and verify root matches
        let mut new_db = InMemoryStateDB::new();
        let result = SnapshotApplier::apply(&mut new_db, &snapshot).unwrap();
        assert_eq!(result.state_root, original_root);
    }

    #[test]
    fn test_tampered_snapshot_rejected() {
        let mut db = InMemoryStateDB::new();
        populate_db(&mut db);

        let mut snapshot = SnapshotBuilder::create(&mut db, 100, 5).unwrap();

        // Tamper with an account balance
        snapshot.accounts[0].balance = 999_999_999;

        let mut new_db = InMemoryStateDB::new();
        let result = SnapshotApplier::apply(&mut new_db, &snapshot);
        // Body hash or state root mismatch
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_snapshot() {
        let mut db = InMemoryStateDB::new();
        let snapshot = SnapshotBuilder::create(&mut db, 0, 0).unwrap();

        assert_eq!(snapshot.header.account_count, 0);
        assert_eq!(snapshot.header.object_count, 0);
        assert_eq!(snapshot.header.ghost_count, 0);

        // Apply empty snapshot
        let mut new_db = InMemoryStateDB::new();
        let result = SnapshotApplier::apply(&mut new_db, &snapshot).unwrap();
        assert_eq!(result.accounts_restored, 0);
    }

    #[test]
    fn test_serialize_deserialize_roundtrip() {
        let mut db = InMemoryStateDB::new();
        populate_db(&mut db);

        let snapshot = SnapshotBuilder::create(&mut db, 100, 5).unwrap();
        let bytes = serialize_snapshot(&snapshot).unwrap();
        let decoded = deserialize_snapshot(&bytes).unwrap();

        assert_eq!(decoded.header.block_height, 100);
        assert_eq!(decoded.header.state_root, snapshot.header.state_root);
        assert_eq!(decoded.header.body_hash, snapshot.header.body_hash);
        assert_eq!(decoded.accounts.len(), snapshot.accounts.len());
        assert_eq!(decoded.objects.len(), snapshot.objects.len());
        assert_eq!(decoded.ghosts.len(), snapshot.ghosts.len());
    }

    #[test]
    fn test_snapshot_diff_detects_changes() {
        let mut db1 = InMemoryStateDB::new();
        db1.put_account(make_account(1, 1000));
        db1.put_account(make_account(2, 2000));
        db1.put_object(make_object(1, 100));
        let snap1 = SnapshotBuilder::create(&mut db1, 1, 0).unwrap();

        let mut db2 = InMemoryStateDB::new();
        db2.put_account(make_account(1, 1500)); // changed balance
        db2.put_account(make_account(3, 3000)); // new account
        // account 2 removed
        db2.put_object(make_object(1, 100)); // unchanged
        db2.put_object(make_object(2, 200)); // new object
        let snap2 = SnapshotBuilder::create(&mut db2, 2, 0).unwrap();

        let diff = SnapshotDiff::compute(&snap1, &snap2);

        assert_eq!(diff.base_height, 1);
        assert_eq!(diff.target_height, 2);
        assert_eq!(diff.accounts_changed.len(), 2); // account 1 (changed) + account 3 (new)
        assert_eq!(diff.accounts_removed.len(), 1); // account 2
        assert_eq!(diff.objects_changed.len(), 1);  // object 2 (new)
        assert_eq!(diff.objects_removed.len(), 0);
        assert!(!diff.is_empty());
    }

    #[test]
    fn test_snapshot_diff_empty_when_identical() {
        let mut db = InMemoryStateDB::new();
        populate_db(&mut db);

        let snap1 = SnapshotBuilder::create(&mut db, 1, 0).unwrap();
        let snap2 = SnapshotBuilder::create(&mut db, 2, 0).unwrap();

        let diff = SnapshotDiff::compute(&snap1, &snap2);
        assert!(diff.is_empty());
        assert_eq!(diff.total_changes(), 0);
    }

    #[test]
    fn test_snapshot_diff_ghost_tracking() {
        let mut db1 = InMemoryStateDB::new();
        db1.put_object(make_object(1, 100));
        let snap1 = SnapshotBuilder::create(&mut db1, 1, 0).unwrap();

        // Object 1 evaporated → became ghost
        let mut db2 = InMemoryStateDB::new();
        db2.put_ghost(make_ghost(1, 10));
        let snap2 = SnapshotBuilder::create(&mut db2, 2, 1).unwrap();

        let diff = SnapshotDiff::compute(&snap1, &snap2);
        assert_eq!(diff.objects_removed.len(), 1); // object 1 gone from active
        assert_eq!(diff.ghosts_added.len(), 1);     // ghost 1 added
    }

    #[test]
    fn test_snapshot_with_privacy_state() {
        let mut db = InMemoryStateDB::new();
        db.put_account(make_account(1, 1000));
        db.put_note_tree_root([0xAB; 32]);
        db.put_shielded_pool_balance(50_000);
        db.put_note_count(42);
        db.spend_nullifier(&[0x01; 32]);
        db.spend_nullifier(&[0x02; 32]);

        let snapshot = SnapshotBuilder::create(&mut db, 10, 1).unwrap();

        assert_eq!(snapshot.privacy.note_tree_root, [0xAB; 32]);
        assert_eq!(snapshot.privacy.shielded_pool_balance, 50_000);
        assert_eq!(snapshot.privacy.note_count, 42);

        // Apply and verify privacy state restored
        let mut new_db = InMemoryStateDB::new();
        SnapshotApplier::apply(&mut new_db, &snapshot).unwrap();

        assert_eq!(new_db.get_note_tree_root(), [0xAB; 32]);
        assert_eq!(new_db.get_shielded_pool_balance(), 50_000);
        assert_eq!(new_db.get_note_count(), 42);
    }

    #[test]
    fn test_version_mismatch_rejected() {
        let mut db = InMemoryStateDB::new();
        db.put_account(make_account(1, 1000));

        let mut snapshot = SnapshotBuilder::create(&mut db, 1, 0).unwrap();
        snapshot.header.version = 999;

        let mut new_db = InMemoryStateDB::new();
        let result = SnapshotApplier::apply(&mut new_db, &snapshot);
        assert!(matches!(result, Err(SnapshotError::VersionMismatch { .. })));
    }

    #[test]
    fn test_body_hash_deterministic() {
        let mut db = InMemoryStateDB::new();
        populate_db(&mut db);

        let snap1 = SnapshotBuilder::create(&mut db, 100, 5).unwrap();
        let snap2 = SnapshotBuilder::create(&mut db, 100, 5).unwrap();

        assert_eq!(snap1.header.body_hash, snap2.header.body_hash);
        assert_eq!(snap1.header.state_root, snap2.header.state_root);
    }

    #[test]
    fn test_large_snapshot() {
        let mut db = InMemoryStateDB::new();

        // Create 100 accounts and 50 objects
        for i in 0..100u8 {
            db.put_account(Account {
                address: {
                    let mut a = [0u8; 32];
                    a[0] = i;
                    a[1] = (i / 10) + 1;
                    a
                },
                balance: (i as u64 + 1) * 10_000,
                nonce: i as u64,
            });
        }
        for i in 0..50u8 {
            db.put_object(make_object(i, (i as u64 + 1) * 100));
        }
        for i in 50..60u8 {
            db.put_ghost(make_ghost(i, i as u64));
        }

        let snapshot = SnapshotBuilder::create(&mut db, 500, 25).unwrap();
        assert_eq!(snapshot.header.account_count, 100);
        assert_eq!(snapshot.header.object_count, 50);
        assert_eq!(snapshot.header.ghost_count, 10);

        // Serialize and deserialize
        let bytes = serialize_snapshot(&snapshot).unwrap();
        let decoded = deserialize_snapshot(&bytes).unwrap();
        assert_eq!(decoded.header.account_count, 100);

        // Apply to fresh DB
        let mut new_db = InMemoryStateDB::new();
        let result = SnapshotApplier::apply(&mut new_db, &decoded).unwrap();
        assert_eq!(result.accounts_restored, 100);
        assert_eq!(result.objects_restored, 50);
        assert_eq!(result.state_root, snapshot.header.state_root);
    }
}
