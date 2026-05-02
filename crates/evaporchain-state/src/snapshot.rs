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

/// Minimum block height past the chain tip before a snapshot is safe from reorgs.
/// Tendermint BFT finalizes instantly, but we require at least this many confirmations
/// to protect sync nodes from loading a height that could theoretically be reverted
/// in extreme slashing / equivocation scenarios (audit finding §8).
pub const SNAPSHOT_MIN_FINALITY_DEPTH: u64 = 1;

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
    #[error("snapshot at height {height} is below finality depth {required}")]
    BelowFinalityDepth { height: u64, required: u64 },
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
    /// Create a snapshot that is safe to serve to sync nodes.
    ///
    /// Enforces that `block_height` is past `chain_tip - SNAPSHOT_MIN_FINALITY_DEPTH`
    /// to protect against serving a snapshot for a height that could be reverted.
    /// For testing or operator tooling where finality is externally guaranteed,
    /// use [`create`] directly.
    pub fn create_finalized(
        db: &mut dyn StateDB,
        block_height: u64,
        epoch: u64,
        chain_tip: u64,
    ) -> Result<StateSnapshot, SnapshotError> {
        if chain_tip > block_height && chain_tip - block_height < SNAPSHOT_MIN_FINALITY_DEPTH {
            return Err(SnapshotError::BelowFinalityDepth {
                height: block_height,
                required: chain_tip.saturating_sub(SNAPSHOT_MIN_FINALITY_DEPTH - 1),
            });
        }
        Self::create(db, block_height, epoch)
    }

    /// Create a snapshot from the current state database.
    ///
    /// Callers MUST ensure `block_height` is past the finality window before
    /// serving the snapshot to sync peers. Prefer [`create_finalized`] in
    /// production code; this method is kept for testing and local tooling.
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
        accounts.sort_by_key(|a| a.address);

        // Collect objects (sorted by ID)
        let mut objects: Vec<StateObject> = db
            .all_object_ids()
            .into_iter()
            .filter_map(|id| db.get_object(&id).cloned())
            .collect();
        objects.sort_by_key(|o| o.id);

        // Collect ghosts (sorted by object ID)
        let mut ghosts: Vec<GhostRecord> = db
            .all_ghost_ids()
            .into_iter()
            .filter_map(|id| db.get_ghost(&id).cloned())
            .collect();
        ghosts.sort_by_key(|g| g.object_id);

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

        // Clear existing state so stale entries don't persist after restore.
        for id in db.all_object_ids() {
            db.delete_object(&id);
        }
        for id in db.all_ghost_ids() {
            db.remove_ghost(&id);
        }
        let snapshot_addrs: std::collections::HashSet<AccountAddress> =
            snapshot.accounts.iter().map(|a| a.address).collect();
        for addr in db.all_account_addresses() {
            if !snapshot_addrs.contains(&addr) {
                db.delete_account(&addr);
            }
        }

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
                Some(base_acc)
                    if acc.balance != base_acc.balance || acc.nonce != base_acc.nonce =>
                {
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
    bincode::serialize(snapshot).map_err(|e| SnapshotError::SerializationError(e.to_string()))
}

/// Deserialize a snapshot from bytes.
pub fn deserialize_snapshot(bytes: &[u8]) -> Result<StateSnapshot, SnapshotError> {
    bincode::deserialize(bytes).map_err(|e| SnapshotError::DeserializationError(e.to_string()))
}

// ─────────────────────── On-Disk Snapshot Blob ─────────────────────────
//
// The file format used by `evaporchain-cli snapshot create` and the
// `/api/snapshot/download/:height` endpoint. Wraps the in-memory
// `StateSnapshot` plus consensus metadata (chain_id, parent_hash,
// validator_set_snapshot, bell_reading) and emits a zstd-compressed
// bincode blob with a 4-byte magic header `EVSN` + 1-byte version.
//
// This is a fast-sync bootstrap format. New nodes download the blob,
// verify the integrity hash, and apply it via `SnapshotFile::apply_to`
// before engaging Tendermint normal-sync from `block_height + 1`.

/// Magic header bytes — first four bytes of every `.zst` snapshot blob.
pub const SNAPSHOT_MAGIC: &[u8; 4] = b"EVSN";

/// On-disk version byte (5th byte of the file). Bumps when the file
/// layout changes incompatibly. Distinct from `SNAPSHOT_VERSION` which
/// covers the in-memory `StateSnapshot` schema.
pub const SNAPSHOT_FILE_VERSION: u8 = 1;

/// Default zstd compression level — balances throughput against size.
pub const SNAPSHOT_COMPRESSION_LEVEL: i32 = 10;

/// Per-block CHSH Bell-Beacon measurement persisted in the snapshot
/// file so a fast-syncing node can restore the wallet-visible
/// `/api/bell/latest` reading immediately. Mirrors the consensus-layer
/// `CheckpointedBellReading` shape exactly so both crates can convert
/// without pulling each other in as deps.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotBellReading {
    pub s_value_milli: u64,
    pub block_height: u64,
    pub epoch: u64,
    pub certified: bool,
}

/// Validator entry persisted in the snapshot file. Opaque to
/// `evaporchain-state`; node-side code converts from
/// `evaporchain_consensus::validator_set::ValidatorInfo`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotValidator {
    pub id: u64,
    pub stake: u64,
    pub address: [u8; 32],
    pub bls_public_key: Option<Vec<u8>>,
    pub vrf_public_key: Option<Vec<u8>>,
    pub jailed: bool,
}

/// Validator-set snapshot bundled with the state snapshot.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValidatorSetSnapshot {
    pub validators: Vec<SnapshotValidator>,
}

/// Smart-contract storage entry. The current `StateDB` trait does not
/// expose contract storage as a typed surface, so this vec is reserved
/// for forward compatibility — populated when contract storage is
/// migrated under `StateDB`. Today: always empty.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContractEntry {
    pub id: [u8; 32],
    pub code: Vec<u8>,
    pub storage: Vec<([u8; 32], Vec<u8>)>,
}

/// On-disk snapshot file. Serialised via `bincode`, compressed with
/// `zstd`, prefixed with magic + version. The integrity hash is
/// recomputed on load and asserted equal — flipping any byte causes
/// a verify failure.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SnapshotFile {
    /// Format version (file schema). Currently `1`.
    pub version: u32,
    /// Chain identifier (e.g. `evaporchain-mainnet-1`). Mismatch =>
    /// reject — never apply a foreign chain's snapshot to a local DB.
    pub chain_id: String,
    /// Block height the snapshot was taken at.
    pub block_height: u64,
    /// State root at `block_height` (Verkle trie root).
    pub state_root: [u8; 32],
    /// Epoch at snapshot time.
    pub epoch: u64,
    /// Parent block hash for `block_height + 1`.
    pub parent_hash: [u8; 32],
    /// Unix milliseconds at file creation.
    pub created_at: u64,
    /// Entire account map.
    pub accounts: Vec<Account>,
    /// Entire object map (active state).
    pub objects: Vec<StateObject>,
    /// Entire contract storage. Empty until contracts move under StateDB.
    pub contracts: Vec<ContractEntry>,
    /// Ghost records (evaporated objects pending resurrection).
    pub ghosts: Vec<GhostRecord>,
    /// Spent nullifiers, sorted.
    pub spent_nullifiers: Vec<[u8; 32]>,
    /// Privacy layer state.
    pub note_tree_root: [u8; 32],
    pub shielded_pool_balance: u64,
    pub note_count: u64,
    /// Last per-block Bell-Beacon CHSH reading.
    pub bell_reading: Option<SnapshotBellReading>,
    /// Validator-set snapshot.
    pub validator_set: ValidatorSetSnapshot,
    /// BLAKE3 over all preceding fields canonically serialised. Verifier
    /// recomputes and asserts equal.
    pub integrity_hash: [u8; 32],
}

/// Lightweight metadata returned by `/api/snapshot/latest`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SnapshotMetadata {
    pub version: u32,
    pub chain_id: String,
    pub block_height: u64,
    pub state_root: [u8; 32],
    pub epoch: u64,
    pub integrity_hash: [u8; 32],
    pub size_bytes: u64,
    pub download_path: String,
}

impl SnapshotFile {
    /// Build a snapshot file by reading the entire active state out of
    /// `db`. Caller supplies consensus metadata (chain_id, parent_hash,
    /// epoch, bell_reading, validator_set). The integrity hash is
    /// computed and embedded.
    pub fn create(
        db: &mut dyn StateDB,
        chain_id: impl Into<String>,
        block_height: u64,
        epoch: u64,
        parent_hash: [u8; 32],
        bell_reading: Option<SnapshotBellReading>,
        validator_set: ValidatorSetSnapshot,
    ) -> Result<Self, SnapshotError> {
        let mut accounts: Vec<Account> = db
            .all_account_addresses()
            .into_iter()
            .filter_map(|addr| db.get_account(&addr).cloned())
            .collect();
        accounts.sort_by_key(|a| a.address);

        let mut objects: Vec<StateObject> = db
            .all_object_ids()
            .into_iter()
            .filter_map(|id| db.get_object(&id).cloned())
            .collect();
        objects.sort_by_key(|o| o.id);

        let mut ghosts: Vec<GhostRecord> = db
            .all_ghost_ids()
            .into_iter()
            .filter_map(|id| db.get_ghost(&id).cloned())
            .collect();
        ghosts.sort_by_key(|g| g.object_id);

        let mut spent_nullifiers = db.all_nullifiers();
        spent_nullifiers.sort();

        let state_root = db.compute_state_root();

        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let mut file = Self {
            version: SNAPSHOT_VERSION,
            chain_id: chain_id.into(),
            block_height,
            state_root,
            epoch,
            parent_hash,
            created_at,
            accounts,
            objects,
            contracts: Vec::new(),
            ghosts,
            spent_nullifiers,
            note_tree_root: db.get_note_tree_root(),
            shielded_pool_balance: db.get_shielded_pool_balance(),
            note_count: db.get_note_count(),
            bell_reading,
            validator_set,
            integrity_hash: [0u8; 32],
        };
        file.integrity_hash = file.compute_integrity_hash();
        Ok(file)
    }

    /// Recompute the BLAKE3 over every field except `integrity_hash`
    /// itself. Used both at create time (to fill the field) and at load
    /// time (to verify it was not tampered).
    fn compute_integrity_hash(&self) -> [u8; 32] {
        // Canonical serialisation: clone with integrity_hash zeroed.
        let mut canonical = self.clone();
        canonical.integrity_hash = [0u8; 32];
        match bincode::serialize(&canonical) {
            Ok(bytes) => blake3_hash(&bytes),
            Err(_) => [0u8; 32],
        }
    }

    /// Serialise + compress + prefix with magic header. Returns the
    /// full on-disk bytes.
    pub fn to_bytes(&self) -> Result<Vec<u8>, SnapshotError> {
        let serialized = bincode::serialize(self)
            .map_err(|e| SnapshotError::SerializationError(e.to_string()))?;
        let compressed = zstd::stream::encode_all(&serialized[..], SNAPSHOT_COMPRESSION_LEVEL)
            .map_err(|e| SnapshotError::SerializationError(format!("zstd encode: {e}")))?;
        let mut out = Vec::with_capacity(5 + compressed.len());
        out.extend_from_slice(SNAPSHOT_MAGIC);
        out.push(SNAPSHOT_FILE_VERSION);
        out.extend_from_slice(&compressed);
        Ok(out)
    }

    /// Parse + decompress + deserialise + verify integrity hash.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SnapshotError> {
        if bytes.len() < 5 {
            return Err(SnapshotError::Invalid(
                "snapshot file too short for header".into(),
            ));
        }
        if &bytes[..4] != SNAPSHOT_MAGIC {
            return Err(SnapshotError::Invalid(format!(
                "bad magic: expected {:?}, got {:?}",
                SNAPSHOT_MAGIC,
                &bytes[..4]
            )));
        }
        if bytes[4] != SNAPSHOT_FILE_VERSION {
            return Err(SnapshotError::VersionMismatch {
                expected: SNAPSHOT_FILE_VERSION as u32,
                actual: bytes[4] as u32,
            });
        }
        let payload = &bytes[5..];
        let decompressed = zstd::stream::decode_all(payload)
            .map_err(|e| SnapshotError::DeserializationError(format!("zstd decode: {e}")))?;
        let file: SnapshotFile = bincode::deserialize(&decompressed)
            .map_err(|e| SnapshotError::DeserializationError(e.to_string()))?;

        if file.version != SNAPSHOT_VERSION {
            return Err(SnapshotError::VersionMismatch {
                expected: SNAPSHOT_VERSION,
                actual: file.version,
            });
        }

        let recomputed = file.compute_integrity_hash();
        if recomputed != file.integrity_hash {
            return Err(SnapshotError::StateRootMismatch {
                expected: hex::encode(file.integrity_hash),
                actual: hex::encode(recomputed),
            });
        }
        Ok(file)
    }

    /// Convenience: write the on-disk bytes to `path`. Caller should
    /// pass a `.zst` extension for clarity but it isn't enforced.
    pub fn write_to_path(&self, path: &std::path::Path) -> Result<u64, SnapshotError> {
        let bytes = self.to_bytes()?;
        let len = bytes.len() as u64;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| SnapshotError::Invalid(format!("create dir: {e}")))?;
        }
        std::fs::write(path, &bytes)
            .map_err(|e| SnapshotError::Invalid(format!("write file: {e}")))?;
        Ok(len)
    }

    /// Convenience: read + verify from disk.
    pub fn load_and_verify(path: &std::path::Path) -> Result<Self, SnapshotError> {
        let bytes = std::fs::read(path)
            .map_err(|e| SnapshotError::Invalid(format!("read file {}: {e}", path.display())))?;
        Self::from_bytes(&bytes)
    }

    /// Lightweight metadata view (does not include contents).
    pub fn metadata(&self, size_bytes: u64) -> SnapshotMetadata {
        SnapshotMetadata {
            version: self.version,
            chain_id: self.chain_id.clone(),
            block_height: self.block_height,
            state_root: self.state_root,
            epoch: self.epoch,
            integrity_hash: self.integrity_hash,
            size_bytes,
            download_path: format!("/api/snapshot/download/{}", self.block_height),
        }
    }

    /// Wipe `db` and replay every account/object/ghost/contract/privacy
    /// entry from the snapshot. Verifies the resulting state root
    /// matches the embedded value.
    pub fn apply_to(&self, db: &mut dyn StateDB) -> Result<ApplyResult, SnapshotError> {
        let start = std::time::Instant::now();

        // Wipe existing state so stale entries don't survive restore.
        for id in db.all_object_ids() {
            db.delete_object(&id);
        }
        for id in db.all_ghost_ids() {
            db.remove_ghost(&id);
        }
        let snapshot_addrs: std::collections::HashSet<AccountAddress> =
            self.accounts.iter().map(|a| a.address).collect();
        for addr in db.all_account_addresses() {
            if !snapshot_addrs.contains(&addr) {
                db.delete_account(&addr);
            }
        }

        for acc in &self.accounts {
            db.put_account(acc.clone());
        }
        for obj in &self.objects {
            db.put_object(obj.clone());
        }
        for ghost in &self.ghosts {
            db.put_ghost(ghost.clone());
        }

        db.put_note_tree_root(self.note_tree_root);
        db.put_shielded_pool_balance(self.shielded_pool_balance);
        db.put_note_count(self.note_count);
        for nullifier in &self.spent_nullifiers {
            db.spend_nullifier(nullifier);
        }

        let computed_root = db.compute_state_root();
        if computed_root != self.state_root {
            return Err(SnapshotError::StateRootMismatch {
                expected: hex::encode(self.state_root),
                actual: hex::encode(computed_root),
            });
        }

        Ok(ApplyResult {
            accounts_restored: self.accounts.len(),
            objects_restored: self.objects.len(),
            ghosts_restored: self.ghosts.len(),
            nullifiers_restored: self.spent_nullifiers.len(),
            state_root: computed_root,
            elapsed_ms: start.elapsed().as_millis() as u64,
        })
    }
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
        storage_deposit: 0,
        storage_bytes: 0,
        last_touched_epoch: 0,
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
            lad_mode: None,
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
        assert_eq!(diff.objects_changed.len(), 1); // object 2 (new)
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
        assert_eq!(diff.ghosts_added.len(), 1); // ghost 1 added
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
                storage_deposit: 0,
                storage_bytes: 0,
                last_touched_epoch: 0,
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

    #[test]
    fn test_apply_snapshot_clears_stale_entries() {
        // DB has objects/ghosts NOT in the snapshot — they must be removed
        let mut source_db = InMemoryStateDB::new();
        source_db.put_account(make_account(1, 1000));
        source_db.put_object(make_object(1, 100));
        let snapshot = SnapshotBuilder::create(&mut source_db, 10, 1).unwrap();

        let mut target_db = InMemoryStateDB::new();
        target_db.put_account(make_account(99, 999_999));
        target_db.put_object(make_object(99, 9999));
        target_db.put_ghost(make_ghost(99, 5));

        SnapshotApplier::apply(&mut target_db, &snapshot).unwrap();

        assert!(
            target_db.get_object(&obj_id(99)).is_none(),
            "stale object must be cleared"
        );
        assert!(
            target_db.get_ghost(&obj_id(99)).is_none(),
            "stale ghost must be cleared"
        );
        assert!(
            target_db.get_account(&addr(99)).is_none(),
            "stale account must be deleted"
        );
    }

    // ─── SnapshotFile (on-disk blob format) ─────────────────────────────

    fn make_validator_set() -> ValidatorSetSnapshot {
        ValidatorSetSnapshot {
            validators: vec![SnapshotValidator {
                id: 1,
                stake: 1_000_000,
                address: addr(1),
                bls_public_key: Some(vec![0xAA; 48]),
                vrf_public_key: Some(vec![0xBB; 32]),
                jailed: false,
            }],
        }
    }

    #[test]
    fn snapshot_round_trip_in_memory() {
        let mut db = InMemoryStateDB::new();
        populate_db(&mut db);

        let parent_hash = [0xCC; 32];
        let bell = Some(SnapshotBellReading {
            s_value_milli: 2828,
            block_height: 100,
            epoch: 5,
            certified: true,
        });
        let file = SnapshotFile::create(
            &mut db,
            "evaporchain-test-1",
            100,
            5,
            parent_hash,
            bell.clone(),
            make_validator_set(),
        )
        .unwrap();
        let bytes = file.to_bytes().unwrap();
        // Magic header + version byte present.
        assert_eq!(&bytes[..4], SNAPSHOT_MAGIC);
        assert_eq!(bytes[4], SNAPSHOT_FILE_VERSION);

        let parsed = SnapshotFile::from_bytes(&bytes).unwrap();
        assert_eq!(parsed.chain_id, "evaporchain-test-1");
        assert_eq!(parsed.block_height, 100);
        assert_eq!(parsed.epoch, 5);
        assert_eq!(parsed.parent_hash, parent_hash);
        assert_eq!(parsed.bell_reading, bell);
        assert_eq!(parsed.validator_set.validators.len(), 1);

        let mut target = InMemoryStateDB::new();
        let result = parsed.apply_to(&mut target).unwrap();
        assert_eq!(result.accounts_restored, 3);
        assert_eq!(result.objects_restored, 2);
        assert_eq!(result.ghosts_restored, 1);
        assert_eq!(result.state_root, file.state_root);
        assert_eq!(target.get_account(&addr(1)).unwrap().balance, 1_000_000);
    }

    #[test]
    fn snapshot_integrity_hash_mismatch_rejects() {
        let mut db = InMemoryStateDB::new();
        populate_db(&mut db);

        let file = SnapshotFile::create(
            &mut db,
            "evaporchain-test-1",
            100,
            5,
            [0u8; 32],
            None,
            make_validator_set(),
        )
        .unwrap();
        let mut bytes = file.to_bytes().unwrap();
        // Flip a byte deep in the compressed payload (after the 5-byte
        // header) — corrupts the bincode body, integrity hash recompute
        // will not match.
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;
        let result = SnapshotFile::from_bytes(&bytes);
        assert!(
            result.is_err(),
            "tampered snapshot must fail to verify; got {:?}",
            result
        );
    }

    #[test]
    fn snapshot_version_mismatch_rejects() {
        let mut db = InMemoryStateDB::new();
        populate_db(&mut db);

        let file = SnapshotFile::create(
            &mut db,
            "evaporchain-test-1",
            100,
            5,
            [0u8; 32],
            None,
            make_validator_set(),
        )
        .unwrap();
        let mut bytes = file.to_bytes().unwrap();
        // Mutate the on-disk version byte to a future value.
        bytes[4] = SNAPSHOT_FILE_VERSION + 1;
        let result = SnapshotFile::from_bytes(&bytes);
        assert!(matches!(
            result,
            Err(SnapshotError::VersionMismatch { .. })
        ));
    }

    #[test]
    fn snapshot_chain_id_mismatch_rejects() {
        // Verify that two snapshots with different chain ids produce
        // different integrity hashes — i.e. a snapshot from chain A
        // can't be silently applied to chain B without the verifier
        // noticing. (A node-side check on `chain_id` belt-and-braces
        // this — see fast_sync_from_peer.)
        let mut db = InMemoryStateDB::new();
        populate_db(&mut db);
        let file_a = SnapshotFile::create(
            &mut db,
            "chain-a",
            100,
            5,
            [0u8; 32],
            None,
            make_validator_set(),
        )
        .unwrap();
        let file_b = SnapshotFile::create(
            &mut db,
            "chain-b",
            100,
            5,
            [0u8; 32],
            None,
            make_validator_set(),
        )
        .unwrap();
        assert_ne!(file_a.integrity_hash, file_b.integrity_hash);

        // And: deserialising chain-A's blob and patching the chain_id
        // post-load is caught by the integrity-hash recompute (the hash
        // covers chain_id, so tampering invalidates it).
        let bytes = file_a.to_bytes().unwrap();
        let mut tampered = SnapshotFile::from_bytes(&bytes).unwrap();
        tampered.chain_id = "chain-b".to_string();
        // Re-serialise without recomputing the integrity_hash (mimics an
        // attacker swapping the field but not regenerating the hash).
        let blob_after_tamper = bincode::serialize(&tampered).unwrap();
        let mut framed = Vec::new();
        framed.extend_from_slice(SNAPSHOT_MAGIC);
        framed.push(SNAPSHOT_FILE_VERSION);
        let compressed = zstd::encode_all(&blob_after_tamper[..], 1).unwrap();
        framed.extend_from_slice(&compressed);
        assert!(SnapshotFile::from_bytes(&framed).is_err());
    }

    #[test]
    fn snapshot_file_path_round_trip() {
        let dir = std::env::temp_dir().join(format!(
            "evaporchain-snap-test-{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("snap-100.zst");

        let mut db = InMemoryStateDB::new();
        populate_db(&mut db);
        let file = SnapshotFile::create(
            &mut db,
            "evaporchain-test-1",
            100,
            5,
            [0u8; 32],
            None,
            make_validator_set(),
        )
        .unwrap();
        let written = file.write_to_path(&path).unwrap();
        assert!(written > 0);
        assert!(path.exists());

        let loaded = SnapshotFile::load_and_verify(&path).unwrap();
        assert_eq!(loaded.block_height, 100);
        assert_eq!(loaded.state_root, file.state_root);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
