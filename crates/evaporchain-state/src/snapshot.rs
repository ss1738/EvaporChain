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
use evaporchain_crypto::signatures::{BlsPublicKey, BlsSignature, BlsVerifier};
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
    /// T0.8 sub-task 2: snapshot lacks a quorum certificate but
    /// strict-mode verification requires one.
    #[error("missing quorum certificate")]
    MissingQuorumCert,
    /// Quorum cert's bound integrity_hash does not match the
    /// snapshot's actual integrity_hash. Catches an attacker who
    /// attaches a cert signed against a different snapshot.
    #[error("quorum cert integrity_hash mismatch: cert claims {cert_hash}, snapshot is {snap_hash}")]
    QuorumCertIntegrityHashMismatch {
        cert_hash: String,
        snap_hash: String,
    },
    /// Quorum cert's signer-stake sum is below the 2f+1 threshold of
    /// the validator-set total stake.
    #[error("quorum cert below 2f+1 stake: signing={signing}, total={total}")]
    QuorumCertInsufficientStake { signing: u128, total: u128 },
    /// One or more validators named in the cert lack a BLS public key
    /// on file. The cert cannot be cryptographically verified.
    #[error("quorum cert names {missing} validator(s) without bls_public_key")]
    QuorumCertMissingValidatorBlsKey { missing: usize },
    /// Quorum cert names a validator not in the snapshot's validator_set.
    #[error("quorum cert names unknown validator id: {0}")]
    QuorumCertUnknownValidator(u64),
    /// BLS aggregate signature verification on the integrity_hash failed.
    #[error("quorum cert BLS aggregate verify failed")]
    QuorumCertBlsFailed,
    /// Snapshot's validator_set contains two or more entries with the
    /// same validator id. Structurally malformed — `from_bytes`
    /// rejects to prevent downstream consensus / slashing code from
    /// observing the duplicate-id ambiguity.
    #[error("validator_set has duplicate validator id: {0}")]
    DuplicateValidatorId(u64),
    /// Snapshot's accounts list contains two or more entries with the
    /// same address. Structurally malformed. Per-address state is
    /// supposed to be unique.
    #[error("accounts list has duplicate address: {0}")]
    DuplicateAccountAddress(String),
    /// Snapshot's objects list contains two or more entries with the
    /// same object id. Same shape as the duplicate-account check.
    #[error("objects list has duplicate id: {0}")]
    DuplicateObjectId(String),
    /// M12 (audit 2026-05-13): the snapshot apply path now wraps the
    /// wipe + repopulate in a state batch. If the final
    /// `commit_batch` fails, surface the WriteBatch error rather
    /// than leaving a half-applied DB in place.
    #[error("snapshot batch commit failed: {0}")]
    CommitFailed(String),
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

        // M12 (audit 2026-05-13): snapshot apply is now ATOMIC and
        // CLEAN-SLATE.
        //
        // Pre-fix this path only wiped objects/ghosts/accounts —
        // stakes, delegations, sentinel params/votes, note
        // commitments, prior spent nullifiers, vesting schedules, and
        // governance state all survived a restore. That left a hybrid
        // state where slashing kept hitting ghost validator stakes
        // and previously spent nullifiers blocked legitimate notes.
        //
        // wipe_full_state_for_snapshot_restore() blasts every state
        // CF the snapshot is about to repopulate. The snapshot format
        // does NOT carry stakes/delegations/sentinel — this path is
        // therefore strictly "first-time join / cold-start" semantics:
        // the joining node ends up with empty stakes/delegations/
        // sentinel state and must learn them from block replay.
        // Callers must NOT invoke snapshot apply on a node mid-life
        // expecting it to merge.
        //
        // Atomicity: bracket the wipe+repopulate in begin_batch /
        // commit_batch so a panic mid-apply rolls back via
        // rollback_batch instead of leaving a half-restored DB. The
        // state-root check before commit catches snapshot/local-root
        // divergence and discards the speculative apply.
        db.begin_batch();
        db.wipe_full_state_for_snapshot_restore();

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

        // Verify state root matches after apply — rollback the batch
        // before bubbling up so a mismatched snapshot leaves zero
        // mutation visible on the DB.
        let computed_root = db.compute_state_root();
        if computed_root != snapshot.header.state_root {
            db.rollback_batch();
            return Err(SnapshotError::StateRootMismatch {
                expected: hex::encode(snapshot.header.state_root),
                actual: hex::encode(computed_root),
            });
        }
        db.commit_batch().map_err(SnapshotError::CommitFailed)?;

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
    /// T0.8 sub-task 2 — quorum certificate binding the snapshot to a
    /// 2f+1-attestation by the validator set named in `validator_set`.
    /// Optional for backwards compatibility with snapshots built before
    /// this defense landed; strict-mode verification (`from_bytes_strict`)
    /// requires it. The cert signs over `integrity_hash`, so the
    /// `compute_integrity_hash` recipe must EXCLUDE this field to keep
    /// the integrity_hash stable before/after cert attachment.
    ///
    /// NOTE: deliberately NO `skip_serializing_if` — bincode 1.3.3
    /// does not honor skip-when-None for Option fields (writes 0
    /// bytes on serialize but reads 1 byte on deserialize → EOF
    /// error). Account.vesting + paymaster Day-1 hit the same trap.
    /// Always emit the 1-byte Option tag.
    #[serde(default)]
    pub quorum_cert: Option<SnapshotQuorumCert>,
}

/// T0.8 sub-task 2 — snapshot quorum certificate. Validators sign the
/// snapshot's `integrity_hash` (NOT a derived re-hash — the canonical
/// bytes-to-sign are exactly the 32 bytes of `SnapshotFile.integrity_hash`).
/// The aggregate signature must be from at least 2f+1 stake-weighted
/// validators of the snapshot's own `validator_set`.
///
/// Closes the documented-gap test
/// `adversarial_t08_forged_integrity_hash_matches_tampered_bytes`:
/// without this binding, an attacker who controls bytes AND can
/// re-hash gets through the integrity check. With this binding, the
/// attacker would ALSO need 2f+1 BLS signatures over their forged
/// integrity_hash — economically infeasible.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotQuorumCert {
    /// The integrity_hash that was signed. MUST match
    /// `SnapshotFile.integrity_hash` at verify time.
    pub integrity_hash: [u8; 32],
    /// Aggregated BLS12-381 signature (96 bytes) over `integrity_hash`.
    pub aggregate_signature: Vec<u8>,
    /// Validator IDs whose individual signatures were aggregated. Order
    /// is informational; verification de-duplicates via the validator-set
    /// lookup.
    pub signer_ids: Vec<u64>,
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
            // Cert is attached later via `attach_quorum_cert` once
            // 2f+1 validators have signed the freshly-computed
            // integrity_hash. Created-with-None is the canonical
            // shape; strict-mode loaders will reject until the cert
            // is sealed.
            quorum_cert: None,
        };
        file.integrity_hash = file.compute_integrity_hash();
        Ok(file)
    }

    /// Recompute the BLAKE3 over every field except `integrity_hash`
    /// itself AND `created_at`. Used both at create time (to fill the
    /// field) and at load time (to verify it was not tampered).
    ///
    /// `created_at` is excluded from the canonical hash because it's
    /// `SystemTime::now()` at create time — every peer's commit handler
    /// fires at a slightly different wall-clock instant, so including
    /// it makes the integrity_hash diverge across peers even when the
    /// underlying state is byte-identical. With it excluded, two peers
    /// holding the same `block_height + state_root + validator_set + ...`
    /// produce the same `integrity_hash`, which lets a fast-syncing
    /// joiner verify it received a snapshot matching a quorum of peers'
    /// reported hashes (Track-1 follow-up to the 2026-05-02 fast-sync
    /// validation).
    fn compute_integrity_hash(&self) -> [u8; 32] {
        let mut canonical = self.clone();
        canonical.integrity_hash = [0u8; 32];
        canonical.created_at = 0;
        // T0.8 sub-task 2: exclude `quorum_cert` from the integrity
        // hash so the cert can be attached AFTER the snapshot is
        // produced without invalidating the hash. The cert signs over
        // `integrity_hash`; recursive inclusion would create a
        // chicken-and-egg cycle.
        canonical.quorum_cert = None;
        match bincode::serialize(&canonical) {
            Ok(bytes) => blake3_hash(&bytes),
            Err(_) => [0u8; 32],
        }
    }

    /// T0.8 sub-task 2 — verify the snapshot's quorum certificate.
    /// Returns `Ok(())` when the cert is present, binds to this
    /// snapshot's integrity_hash, names enough stake to clear the
    /// 2f+1 threshold of the snapshot's own validator_set, and the
    /// BLS aggregate signature verifies. Any failure returns a
    /// specific `SnapshotError` variant naming the failure mode.
    ///
    /// This is the strict-mode verifier. `from_bytes` does not call
    /// this — backwards-compat retained for snapshots produced before
    /// this defense landed. Operators wanting strict acceptance use
    /// `from_bytes_strict` (which composes from_bytes + this method).
    pub fn verify_quorum_cert(&self) -> Result<(), SnapshotError> {
        let cert = self
            .quorum_cert
            .as_ref()
            .ok_or(SnapshotError::MissingQuorumCert)?;

        // 1. Binding: cert.integrity_hash must equal snapshot's.
        if cert.integrity_hash != self.integrity_hash {
            return Err(SnapshotError::QuorumCertIntegrityHashMismatch {
                cert_hash: hex::encode(cert.integrity_hash),
                snap_hash: hex::encode(self.integrity_hash),
            });
        }

        // 2. Resolve each signer to a validator with a BLS pubkey.
        let mut pks: Vec<BlsPublicKey> = Vec::with_capacity(cert.signer_ids.len());
        let mut signing_stake: u128 = 0;
        let mut missing_bls = 0usize;
        for sid in &cert.signer_ids {
            let v = self
                .validator_set
                .validators
                .iter()
                .find(|v| v.id == *sid)
                .ok_or(SnapshotError::QuorumCertUnknownValidator(*sid))?;
            match &v.bls_public_key {
                Some(b) => pks.push(BlsPublicKey(b.clone())),
                None => {
                    missing_bls += 1;
                }
            }
            signing_stake = signing_stake.saturating_add(v.stake as u128);
        }
        if missing_bls > 0 {
            return Err(SnapshotError::QuorumCertMissingValidatorBlsKey {
                missing: missing_bls,
            });
        }

        // 3. 2f+1 stake-weighted threshold: signing_stake > 2/3 of
        //    total (strict). signing * 3 > total * 2.
        let total_stake: u128 = self
            .validator_set
            .validators
            .iter()
            .map(|v| v.stake as u128)
            .sum();
        if signing_stake.saturating_mul(3) <= total_stake.saturating_mul(2) {
            return Err(SnapshotError::QuorumCertInsufficientStake {
                signing: signing_stake,
                total: total_stake,
            });
        }

        // 4. BLS aggregate verify over the integrity_hash.
        let agg_sig = BlsSignature(cert.aggregate_signature.clone());
        if !BlsVerifier::aggregate_verify(&cert.integrity_hash, &agg_sig, &pks) {
            return Err(SnapshotError::QuorumCertBlsFailed);
        }

        Ok(())
    }

    /// T0.8 sub-task 2 — strict-mode `from_bytes` that ALSO requires
    /// a valid quorum certificate. Use this on the fast-sync hot path
    /// where forged integrity_hash defense matters; the regular
    /// `from_bytes` is retained for tooling / pre-cert legacy snapshots.
    pub fn from_bytes_strict(bytes: &[u8]) -> Result<Self, SnapshotError> {
        let file = Self::from_bytes(bytes)?;
        file.verify_quorum_cert()?;
        Ok(file)
    }

    /// T0.8 follow-on — structural validation of the snapshot's
    /// content vectors. Currently checks:
    ///   - No duplicate validator IDs in validator_set
    ///   - No duplicate account addresses in accounts
    ///   - No duplicate object IDs in objects
    ///
    /// Called by `from_bytes` (and therefore by `from_bytes_strict`)
    /// AFTER integrity-hash verification — by the time we get here
    /// the bytes are self-consistent, so duplicate IDs are a real
    /// structural malformation, not a transit-time tampering issue.
    /// Closes the previously documented gap
    /// `adversarial_t08_duplicate_validator_ids_in_set_accepted_today`.
    fn validate_structure(&self) -> Result<(), SnapshotError> {
        use std::collections::HashSet;

        let mut seen_vid: HashSet<u64> = HashSet::with_capacity(self.validator_set.validators.len());
        for v in &self.validator_set.validators {
            if !seen_vid.insert(v.id) {
                return Err(SnapshotError::DuplicateValidatorId(v.id));
            }
        }

        let mut seen_addr: HashSet<AccountAddress> = HashSet::with_capacity(self.accounts.len());
        for a in &self.accounts {
            if !seen_addr.insert(a.address) {
                return Err(SnapshotError::DuplicateAccountAddress(hex::encode(a.address)));
            }
        }

        let mut seen_oid: HashSet<ObjectId> = HashSet::with_capacity(self.objects.len());
        for o in &self.objects {
            if !seen_oid.insert(o.id) {
                return Err(SnapshotError::DuplicateObjectId(hex::encode(o.id)));
            }
        }

        Ok(())
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
        // Audit (2026-05-18): `zstd::decode_all` is UNBOUNDED — a small
        // highly-compressed blob from a malicious fast-sync peer could
        // expand to OOM the joining node (decompression bomb). Stream
        // with a hard decompressed-size ceiling and reject if exceeded.
        const MAX_DECOMPRESSED_SNAPSHOT_BYTES: u64 = 4 * 1024 * 1024 * 1024; // 4 GiB
        let mut decompressed = Vec::new();
        {
            use std::io::Read;
            let mut zdec = zstd::stream::read::Decoder::new(payload)
                .map_err(|e| SnapshotError::DeserializationError(format!("zstd decoder: {e}")))?;
            (&mut zdec)
                .take(MAX_DECOMPRESSED_SNAPSHOT_BYTES + 1)
                .read_to_end(&mut decompressed)
                .map_err(|e| SnapshotError::DeserializationError(format!("zstd decode: {e}")))?;
        }
        if decompressed.len() as u64 > MAX_DECOMPRESSED_SNAPSHOT_BYTES {
            return Err(SnapshotError::DeserializationError(
                "decompressed snapshot exceeds 4 GiB ceiling (possible decompression bomb)".into(),
            ));
        }
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

        // T0.8 follow-on: structural validation. Catches snapshots
        // whose validator_set / accounts / objects vectors have
        // duplicate IDs. Closes
        // `adversarial_t08_duplicate_validator_ids_in_set_accepted_today`.
        file.validate_structure()?;

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

        // M12 (audit 2026-05-13): clean-slate + atomic restore. See
        // the matching docstring at `SnapshotApplier::apply` for the
        // first-time-join semantics this enforces. The wipe drops
        // stale stakes/delegations/sentinel state/note_commitments
        // that the older partial-wipe path left behind.
        db.begin_batch();
        db.wipe_full_state_for_snapshot_restore();

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
            db.rollback_batch();
            return Err(SnapshotError::StateRootMismatch {
                expected: hex::encode(self.state_root),
                actual: hex::encode(computed_root),
            });
        }
        db.commit_batch().map_err(SnapshotError::CommitFailed)?;

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
            vesting: None,
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
                vesting: None,
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
        assert!(matches!(result, Err(SnapshotError::VersionMismatch { .. })));
    }

    #[test]
    fn snapshot_integrity_hash_excludes_created_at() {
        // Two peers commit the same block at slightly different
        // wall-clock instants. The state and consensus metadata are
        // byte-identical; only `created_at` differs. The integrity_hash
        // MUST match so a fast-syncing joiner can quorum-verify the
        // snapshot it received against `/api/snapshot/latest` from
        // multiple peers (see comment on `compute_integrity_hash`).
        let mut db = InMemoryStateDB::new();
        populate_db(&mut db);

        let mut file_a = SnapshotFile::create(
            &mut db,
            "evaporchain-test-1",
            100,
            5,
            [7u8; 32],
            None,
            make_validator_set(),
        )
        .unwrap();
        let mut file_b = SnapshotFile::create(
            &mut db,
            "evaporchain-test-1",
            100,
            5,
            [7u8; 32],
            None,
            make_validator_set(),
        )
        .unwrap();

        // Force diverging created_at — mimics two peers' commit
        // handlers firing milliseconds apart.
        file_a.created_at = 1_000_000;
        file_b.created_at = 1_000_500;
        file_a.integrity_hash = file_a.compute_integrity_hash();
        file_b.integrity_hash = file_b.compute_integrity_hash();

        assert_eq!(
            file_a.integrity_hash, file_b.integrity_hash,
            "integrity_hash must be reproducible across peers — only \
             `integrity_hash` itself and `created_at` are excluded"
        );

        // Round-trip through bytes: both files must still verify, and
        // both must report the same hash post-load.
        let bytes_a = file_a.to_bytes().unwrap();
        let bytes_b = file_b.to_bytes().unwrap();
        let parsed_a = SnapshotFile::from_bytes(&bytes_a).unwrap();
        let parsed_b = SnapshotFile::from_bytes(&bytes_b).unwrap();
        assert_eq!(parsed_a.integrity_hash, parsed_b.integrity_hash);
        assert_ne!(
            parsed_a.created_at, parsed_b.created_at,
            "created_at should round-trip distinctly even though it \
             doesn't influence the hash"
        );
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

    // ─── Lane T0.8 sub-A — adversarial fast-sync snapshots ─────────
    //
    // Acceptance from MAINNET_READINESS.md T0.8:
    //   "all 5 adversarial fixtures rejected; clean fast-sync still works."
    //
    // This bundle pins five attack vectors against the single-peer
    // snapshot-verification path (`from_bytes` + `apply_to`). Some
    // attacks are caught by existing crypto; one is documented as a
    // KNOWN GAP that requires sub-task 2 (snapshot quorum-cert
    // verification, ≥2f+1 attestations across peers) to close.
    //
    // Reading order (each test is self-explanatory in isolation):
    //   1. truncated_blob       — bytes shorter than the magic header
    //   2. wrong_magic          — magic header replaced with junk
    //   3. state_root_tamper    — integrity_hash recomputed but the
    //                             accounts haven't been changed; apply
    //                             catches the divergence.
    //   4. bell_reading_tamper  — integrity_hash check covers all
    //                             snapshot fields, including consensus
    //                             metadata like the Bell beacon reading.
    //   5. partial_state_with_full_recompute — DOCUMENTED GAP. A
    //      malicious peer that recomputes BOTH the integrity_hash AND
    //      the state_root over a partial account set passes every
    //      internal check. Closing this gap requires sub-task 2.

    #[test]
    fn adversarial_truncated_blob_rejected() {
        // Less than the 5-byte header (magic + version) → from_bytes
        // refuses without even attempting decompression.
        let too_short = vec![0u8; 3];
        let result = SnapshotFile::from_bytes(&too_short);
        assert!(matches!(result, Err(SnapshotError::Invalid(_))));
    }

    #[test]
    fn adversarial_wrong_magic_rejected() {
        // Build a real snapshot, then overwrite the magic prefix with
        // 4 bytes of junk. from_bytes refuses at the magic check (no
        // decompression attempted, no further work done).
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
        bytes[0] = 0xDE;
        bytes[1] = 0xAD;
        bytes[2] = 0xBE;
        bytes[3] = 0xEF;
        let result = SnapshotFile::from_bytes(&bytes);
        assert!(matches!(result, Err(SnapshotError::Invalid(_))));
    }

    #[test]
    fn adversarial_state_root_tamper_caught_by_apply() {
        // Attack: peer rewrites `state_root` to a value of their
        // choice, then recomputes `integrity_hash` so from_bytes
        // accepts. The trap: `apply_to` recomputes the state root
        // from the (untouched) accounts/objects/ghosts and compares
        // against `self.state_root`. The two diverge → reject.
        let mut db = InMemoryStateDB::new();
        populate_db(&mut db);
        let mut file = SnapshotFile::create(
            &mut db,
            "evaporchain-test-1",
            100,
            5,
            [0u8; 32],
            None,
            make_validator_set(),
        )
        .unwrap();

        // Tamper: replace state_root with a fake value AND recompute
        // integrity_hash so the on-disk crypto check passes.
        file.state_root = [0xAB; 32];
        file.integrity_hash = file.compute_integrity_hash();

        // Round-trip — from_bytes accepts the tampered file (the
        // hash check is internally consistent).
        let bytes = file.to_bytes().unwrap();
        let parsed = SnapshotFile::from_bytes(&bytes)
            .expect("internal hash is consistent post-tamper");

        // But apply_to recomputes the state root from the actual
        // restored data and rejects.
        let mut target = InMemoryStateDB::new();
        let result = parsed.apply_to(&mut target);
        match result {
            Err(SnapshotError::StateRootMismatch { expected, actual }) => {
                assert_ne!(expected, actual, "apply must surface the mismatch");
            }
            other => panic!(
                "expected StateRootMismatch from apply_to, got {:?}",
                other
            ),
        }
    }

    #[test]
    fn adversarial_bell_reading_tamper_via_integrity_hash() {
        // Attack: peer alters a consensus-metadata field (`bell_reading`)
        // while leaving the integrity_hash untouched. The hash covers
        // every snapshot field except the hash itself + created_at, so
        // recomputing it on load yields a different value → reject.
        let mut db = InMemoryStateDB::new();
        populate_db(&mut db);
        let original_bell = Some(SnapshotBellReading {
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
            [0u8; 32],
            original_bell,
            make_validator_set(),
        )
        .unwrap();

        // Re-serialise with a swapped bell_reading and the original
        // integrity_hash (mimics a peer who altered metadata but
        // didn't regenerate the hash).
        let bytes = file.to_bytes().unwrap();
        let mut tampered = SnapshotFile::from_bytes(&bytes).unwrap();
        tampered.bell_reading = Some(SnapshotBellReading {
            s_value_milli: 1414, // different reading!
            block_height: 100,
            epoch: 5,
            certified: true,
        });
        // Re-frame without recomputing the integrity_hash.
        let blob_after_tamper = bincode::serialize(&tampered).unwrap();
        let mut framed = Vec::new();
        framed.extend_from_slice(SNAPSHOT_MAGIC);
        framed.push(SNAPSHOT_FILE_VERSION);
        let compressed = zstd::encode_all(&blob_after_tamper[..], 1).unwrap();
        framed.extend_from_slice(&compressed);

        let result = SnapshotFile::from_bytes(&framed);
        assert!(
            matches!(result, Err(SnapshotError::StateRootMismatch { .. })),
            "tampered bell_reading must invalidate the integrity hash; got {:?}",
            result
        );
    }

    /// **DOCUMENTED GAP** — partial-state withholding with full
    /// recomputation. A malicious peer drops several accounts AND
    /// recomputes both the integrity_hash AND the state_root over the
    /// reduced set. Internal checks pass — the on-disk snapshot is
    /// internally consistent. Detection requires comparison against
    /// an EXTERNAL truth source (≥2f+1 peers reporting the same
    /// integrity_hash for that block_height) — this is sub-task 2
    /// of T0.8 (snapshot quorum-cert verification).
    ///
    /// This test pins the gap so reviewers can't accidentally close
    /// T0.8 sub-A without acknowledging that the single-peer trust
    /// model is incomplete on its own. When sub-task 2 lands, this
    /// test should be inverted: the same attack should THEN be
    /// rejected by the quorum-cert check.
    #[test]
    fn adversarial_partial_state_with_full_recompute_passes_single_peer_checks() {
        let mut db = InMemoryStateDB::new();
        populate_db(&mut db); // 3 accounts in the canonical snapshot

        // Honest snapshot — 3 accounts.
        let honest = SnapshotFile::create(
            &mut db,
            "evaporchain-test-1",
            100,
            5,
            [0u8; 32],
            None,
            make_validator_set(),
        )
        .unwrap();

        // Attacker constructs a partial snapshot: 1 account dropped.
        let mut attacker_db = InMemoryStateDB::new();
        attacker_db.put_account(make_account(1, 1_000_000));
        attacker_db.put_account(make_account(2, 500_000));
        // Skip account #3 — the canonical chain has 3 accounts; this
        // peer is serving 2.
        for obj in &honest.objects {
            attacker_db.put_object(obj.clone());
        }
        for ghost in &honest.ghosts {
            attacker_db.put_ghost(ghost.clone());
        }
        attacker_db.put_note_tree_root(honest.note_tree_root);
        attacker_db.put_shielded_pool_balance(honest.shielded_pool_balance);
        attacker_db.put_note_count(honest.note_count);
        for nullifier in &honest.spent_nullifiers {
            attacker_db.spend_nullifier(nullifier);
        }

        let attacker_snapshot = SnapshotFile::create(
            &mut attacker_db,
            "evaporchain-test-1",
            100,
            5,
            [0u8; 32],
            None,
            make_validator_set(),
        )
        .unwrap();

        // The attacker's snapshot has a DIFFERENT integrity_hash
        // and a DIFFERENT state_root from the honest one — but each
        // is internally consistent.
        assert_ne!(
            attacker_snapshot.integrity_hash, honest.integrity_hash,
            "partial-state attack must be detectable by hash compare"
        );
        assert_ne!(attacker_snapshot.state_root, honest.state_root);
        assert_eq!(attacker_snapshot.accounts.len(), 2);
        assert_eq!(honest.accounts.len(), 3);

        // Single-peer fast-sync: the attacker's blob round-trips +
        // applies cleanly. NOTHING in `from_bytes` or `apply_to`
        // detects the divergence — they ONLY check internal
        // consistency.
        let bytes = attacker_snapshot.to_bytes().unwrap();
        let parsed = SnapshotFile::from_bytes(&bytes).expect(
            "attacker's snapshot is internally consistent — single-peer \
             fast-sync has no way to reject it without a quorum cross-check",
        );

        let mut victim_db = InMemoryStateDB::new();
        let result = parsed.apply_to(&mut victim_db).expect(
            "apply succeeds because state_root matches the partial accounts",
        );
        // The victim is now on a divergent state — a chain that diverges
        // from canonical truth at every height ≥100.
        assert_eq!(result.accounts_restored, 2);

        // Acceptance criterion for sub-task 2 (NOT THIS TEST): the
        // joiner queries N peers, sees ≥2f+1 reporting `honest.integrity_hash`
        // for height=100, and refuses to accept any snapshot whose
        // integrity_hash doesn't match the quorum.
    }

    #[test]
    fn snapshot_file_path_round_trip() {
        let dir =
            std::env::temp_dir().join(format!("evaporchain-snap-test-{}", std::process::id()));
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

    // ─── T0.8 sub-task 2 — Quorum-cert binding tests ────────────────

    use evaporchain_crypto::signatures::BlsKeypair;

    /// Helper: build a snapshot with `n` validators, EACH with a real
    /// BLS keypair returned alongside the validator set. The caller
    /// can then sign the integrity_hash with the keypairs of any
    /// subset of signers to produce a cert.
    fn make_snapshot_with_real_bls_validators(n: u64) -> (SnapshotFile, Vec<BlsKeypair>) {
        let mut keypairs: Vec<BlsKeypair> = Vec::with_capacity(n as usize);
        let mut validators: Vec<SnapshotValidator> = Vec::with_capacity(n as usize);
        for i in 1..=n {
            let kp = BlsKeypair::generate();
            let pk_bytes = kp.public_key_bytes().0;
            validators.push(SnapshotValidator {
                id: i,
                stake: 1_000,
                address: addr(i as u8),
                bls_public_key: Some(pk_bytes),
                vrf_public_key: None,
                jailed: false,
            });
            keypairs.push(kp);
        }
        let validator_set = ValidatorSetSnapshot { validators };
        let mut db = InMemoryStateDB::new();
        populate_db(&mut db);
        let file = SnapshotFile::create(
            &mut db,
            "evaporchain-test-1",
            100,
            5,
            [0xCC; 32],
            None,
            validator_set,
        )
        .unwrap();
        (file, keypairs)
    }

    /// Helper: sign `msg` with the given keypairs and aggregate into
    /// a SnapshotQuorumCert.
    fn build_cert(
        msg: &[u8; 32],
        signer_keypairs: &[(&u64, &BlsKeypair)],
    ) -> SnapshotQuorumCert {
        let sigs: Vec<BlsSignature> = signer_keypairs.iter().map(|(_, kp)| kp.sign(msg)).collect();
        let agg = BlsVerifier::aggregate_signatures(&sigs).expect("aggregate non-empty");
        SnapshotQuorumCert {
            integrity_hash: *msg,
            aggregate_signature: agg.0,
            signer_ids: signer_keypairs.iter().map(|(id, _)| **id).collect(),
        }
    }

    #[test]
    fn t08_quorum_cert_happy_path_verifies() {
        let (mut file, kps) = make_snapshot_with_real_bls_validators(4);
        // 3 of 4 sign → 3/4 stake > 2/3 → quorum.
        let ids: Vec<u64> = vec![1, 2, 3];
        let signer_kps: Vec<(&u64, &BlsKeypair)> = ids.iter().zip(kps.iter()).collect();
        file.quorum_cert = Some(build_cert(&file.integrity_hash, &signer_kps));
        let result = file.verify_quorum_cert();
        assert!(result.is_ok(), "happy path must verify: {:?}", result.err());
    }

    #[test]
    fn t08_quorum_cert_missing_returns_missing_quorum_cert() {
        let (file, _kps) = make_snapshot_with_real_bls_validators(4);
        assert!(file.quorum_cert.is_none());
        let err = file.verify_quorum_cert().unwrap_err();
        assert!(matches!(err, SnapshotError::MissingQuorumCert));
    }

    #[test]
    fn t08_quorum_cert_wrong_integrity_hash_rejected() {
        let (mut file, kps) = make_snapshot_with_real_bls_validators(4);
        let wrong_hash = [0xFFu8; 32];
        let ids: Vec<u64> = vec![1, 2, 3];
        let signer_kps: Vec<(&u64, &BlsKeypair)> = ids.iter().zip(kps.iter()).collect();
        file.quorum_cert = Some(build_cert(&wrong_hash, &signer_kps));
        let err = file.verify_quorum_cert().unwrap_err();
        assert!(matches!(
            err,
            SnapshotError::QuorumCertIntegrityHashMismatch { .. }
        ));
    }

    #[test]
    fn t08_quorum_cert_insufficient_stake_rejected() {
        let (mut file, kps) = make_snapshot_with_real_bls_validators(4);
        // Only 2 of 4 sign → 2/4 = 50% < 2/3 → fails threshold.
        let ids: Vec<u64> = vec![1, 2];
        let signer_kps: Vec<(&u64, &BlsKeypair)> = ids.iter().zip(kps.iter()).collect();
        file.quorum_cert = Some(build_cert(&file.integrity_hash, &signer_kps));
        let err = file.verify_quorum_cert().unwrap_err();
        assert!(matches!(
            err,
            SnapshotError::QuorumCertInsufficientStake { .. }
        ));
    }

    #[test]
    fn t08_quorum_cert_unknown_validator_id_rejected() {
        let (mut file, kps) = make_snapshot_with_real_bls_validators(4);
        // Build a cert that names a validator (id 99) that doesn't
        // exist in the snapshot's validator_set.
        let ids: Vec<u64> = vec![1, 2, 99];
        // We can only sign with kps for 1 and 2 (99 doesn't exist).
        // Build cert manually with a placeholder aggregate (won't be
        // BLS-verified — the unknown-validator check fires first).
        let sigs: Vec<BlsSignature> = vec![
            kps[0].sign(&file.integrity_hash),
            kps[1].sign(&file.integrity_hash),
        ];
        let agg = BlsVerifier::aggregate_signatures(&sigs).unwrap();
        file.quorum_cert = Some(SnapshotQuorumCert {
            integrity_hash: file.integrity_hash,
            aggregate_signature: agg.0,
            signer_ids: ids,
        });
        let err = file.verify_quorum_cert().unwrap_err();
        assert!(matches!(
            err,
            SnapshotError::QuorumCertUnknownValidator(99)
        ));
    }

    #[test]
    fn t08_quorum_cert_corrupted_sig_rejected() {
        let (mut file, kps) = make_snapshot_with_real_bls_validators(4);
        let ids: Vec<u64> = vec![1, 2, 3];
        let signer_kps: Vec<(&u64, &BlsKeypair)> = ids.iter().zip(kps.iter()).collect();
        let mut cert = build_cert(&file.integrity_hash, &signer_kps);
        // Flip a byte in the aggregate signature.
        cert.aggregate_signature[0] ^= 0xFF;
        file.quorum_cert = Some(cert);
        let err = file.verify_quorum_cert().unwrap_err();
        assert!(matches!(err, SnapshotError::QuorumCertBlsFailed));
    }

    #[test]
    fn t08_quorum_cert_excluded_from_integrity_hash() {
        // Attaching a cert AFTER snapshot creation must NOT change the
        // integrity_hash — otherwise the cert would invalidate itself.
        let (mut file, kps) = make_snapshot_with_real_bls_validators(4);
        let pre_cert_hash = file.integrity_hash;
        let ids: Vec<u64> = vec![1, 2, 3];
        let signer_kps: Vec<(&u64, &BlsKeypair)> = ids.iter().zip(kps.iter()).collect();
        file.quorum_cert = Some(build_cert(&pre_cert_hash, &signer_kps));
        let post_cert_recomputed = file.compute_integrity_hash();
        assert_eq!(
            pre_cert_hash, post_cert_recomputed,
            "compute_integrity_hash MUST exclude quorum_cert"
        );
    }

    #[test]
    fn t08_from_bytes_strict_requires_cert() {
        let (file, _kps) = make_snapshot_with_real_bls_validators(4);
        let bytes = file.to_bytes().unwrap();
        // No cert attached → strict load rejects.
        let err = SnapshotFile::from_bytes_strict(&bytes).unwrap_err();
        assert!(matches!(err, SnapshotError::MissingQuorumCert));
        // Non-strict load works (backwards compat path).
        assert!(SnapshotFile::from_bytes(&bytes).is_ok());
    }

    #[test]
    fn t08_from_bytes_strict_accepts_valid_cert() {
        let (mut file, kps) = make_snapshot_with_real_bls_validators(4);
        let ids: Vec<u64> = vec![1, 2, 3];
        let signer_kps: Vec<(&u64, &BlsKeypair)> = ids.iter().zip(kps.iter()).collect();
        file.quorum_cert = Some(build_cert(&file.integrity_hash, &signer_kps));
        let bytes = file.to_bytes().unwrap();
        let loaded = SnapshotFile::from_bytes_strict(&bytes).expect("strict load");
        assert_eq!(loaded.block_height, 100);
        assert!(loaded.quorum_cert.is_some());
    }

    // ── M12 (audit 2026-05-13): snapshot apply must wipe stale state ──

    fn make_stake_for_test(validator_id: u64, amount: u64) -> evaporchain_types::StakeRecord {
        evaporchain_types::StakeRecord {
            validator_id,
            validator_address: [validator_id as u8; 32],
            staked_amount: amount,
            staked_at_epoch: 1,
            unbonding_epoch: None,
            slashed_amount: 0,
        }
    }

    fn make_delegation_for_test(
        delegator_byte: u8,
        validator_id: u64,
        amount: u64,
    ) -> evaporchain_types::DelegationRecord {
        evaporchain_types::DelegationRecord {
            delegator: [delegator_byte; 32],
            validator_id,
            amount,
            delegated_at_epoch: 1,
            unbonding_amount: 0,
            unbonding_epoch: None,
        }
    }

    #[test]
    fn audit_m12_apply_wipes_stale_stakes_and_delegations() {
        use evaporchain_sentinel::BoundedParameter;

        let mut db = InMemoryStateDB::new();
        populate_db(&mut db);
        let snapshot = SnapshotBuilder::create(&mut db, 100, 5).unwrap();

        let mut target = InMemoryStateDB::new();
        // Plant stale state that's NOT in the snapshot.
        target.put_stake(make_stake_for_test(99, 999_000));
        target.put_delegation(make_delegation_for_test(0xDD, 99, 111_000));
        target.put_sentinel_param(BoundedParameter::new(77, 7, 0, 100).unwrap());
        assert!(target.get_stake(99).is_some());
        assert!(target.get_sentinel_param(77).is_some());

        SnapshotApplier::apply(&mut target, &snapshot).unwrap();

        // Stale entries gone.
        assert!(target.get_stake(99).is_none());
        assert!(target
            .get_delegation(&[0xDD; 32], 99)
            .is_none());
        assert!(target.get_sentinel_param(77).is_none());
        // Snapshot accounts present.
        assert_eq!(target.get_account(&addr(1)).unwrap().balance, 1_000_000);
    }

    #[test]
    fn audit_m12_apply_wipes_stale_nullifiers_and_note_commitments() {
        let mut db = InMemoryStateDB::new();
        populate_db(&mut db);
        let snapshot = SnapshotBuilder::create(&mut db, 100, 5).unwrap();

        let mut target = InMemoryStateDB::new();
        // Plant nullifiers + note commitments that don't appear in the snapshot.
        let stale_nullifier = [0xCC; 32];
        target.spend_nullifier(&stale_nullifier);
        assert!(target.is_nullifier_spent(&stale_nullifier));
        target.append_note_commitment(0, [0xAA; 32]);
        target.append_note_commitment(1, [0xBB; 32]);
        assert_eq!(target.get_all_note_commitments().len(), 2);

        SnapshotApplier::apply(&mut target, &snapshot).unwrap();

        // The stale nullifier is gone — would have blocked legitimate
        // note spends until the next restart pre-fix.
        assert!(!target.is_nullifier_spent(&stale_nullifier));
        // Note commitments wiped (snapshot had none).
        assert!(target.get_all_note_commitments().is_empty());
    }

    #[test]
    fn audit_m12_state_root_mismatch_rolls_back_all_changes() {
        let mut db = InMemoryStateDB::new();
        populate_db(&mut db);
        let mut snapshot = SnapshotBuilder::create(&mut db, 100, 5).unwrap();

        let mut target = InMemoryStateDB::new();
        // Seed a single pre-existing account so we can prove the
        // wipe + repopulate path either succeeded fully or didn't run
        // at all. Note: SnapshotApplier verifies body_hash first, so
        // tampering with state_root after build still trips the check
        // before any state mutation lands.
        target.put_account(make_account(0x55, 4242));
        // Corrupt the snapshot state root so the post-apply check fails.
        snapshot.header.state_root[0] ^= 0xFF;

        let err = SnapshotApplier::apply(&mut target, &snapshot)
            .expect_err("corrupted state_root must reject");
        match err {
            // body_hash check fires before state_root check; either
            // rejection is acceptable for this regression — the
            // critical post-condition is that pre-existing state is
            // intact.
            SnapshotError::StateRootMismatch { .. } => {}
            other => panic!("unexpected error variant: {other:?}"),
        }
        // Pre-existing account is still there. Atomicity preserved.
        assert_eq!(
            target.get_account(&addr(0x55)).expect("seed acct survives").balance,
            4242
        );
        // Snapshot accounts NOT applied.
        assert!(target.get_account(&addr(1)).is_none());
    }

    #[test]
    fn audit_m12_snapshotfile_apply_to_also_wipes_stale_state() {
        // Belt-and-braces: SnapshotFile::apply_to is the production
        // restore path used by fast-sync. Ensure it wipes too.
        let (file, _kps) = make_snapshot_with_real_bls_validators(2);

        let mut target = InMemoryStateDB::new();
        target.put_stake(make_stake_for_test(123, 456_789));
        SnapshotFile::apply_to(&file, &mut target).unwrap();
        assert!(
            target.get_stake(123).is_none(),
            "fast-sync apply_to must drop stale stakes"
        );
    }
}
