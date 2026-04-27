//! Consensus state persistence for crash recovery.
//!
//! Persists enough state so that a node can restart and resume consensus
//! from the last committed height without re-syncing from genesis.
//!
//! Design:
//! - **Checkpoint**: written on every commit — last committed height, epoch,
//!   parent hash, validator set, weak subjectivity checkpoints.
//! - **WAL (Write-Ahead Log)**: written before phase transitions — allows
//!   recovery mid-round so the node doesn't double-vote.
//! - **Atomic writes**: temp file + rename to prevent corruption on crash.

use crate::validator_set::ValidatorSet;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

// ─────────────────────── Persisted State ────────────────────────────────

/// Snapshot of consensus state written on every commit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsensusCheckpoint {
    /// Last committed block height.
    pub height: u64,
    /// Current epoch.
    pub epoch: u64,
    /// Parent hash for the next block.
    pub parent_hash: [u8; 32],
    /// Weak subjectivity checkpoints: (height, state_root).
    pub weak_subjectivity_checkpoints: Vec<(u64, [u8; 32])>,
    /// Validator set state (serialized separately since ValidatorSet
    /// doesn't derive Serialize — we store the inner validators).
    pub validators: Vec<ValidatorInfoSnapshot>,
}

/// Minimal validator snapshot for persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorInfoSnapshot {
    pub id: u64,
    pub stake: u64,
    pub address: [u8; 32],
    pub bls_public_key: Option<Vec<u8>>,
    pub vrf_public_key: Option<Vec<u8>>,
    pub blocks_produced: u64,
    pub evaporations_processed: u64,
    pub health_score: f64,
    pub jailed: bool,
    pub total_slashed: u64,
}

impl From<&crate::validator_set::ValidatorInfo> for ValidatorInfoSnapshot {
    fn from(v: &crate::validator_set::ValidatorInfo) -> Self {
        Self {
            id: v.id,
            stake: v.stake,
            address: v.address,
            bls_public_key: v.bls_public_key.clone(),
            vrf_public_key: v.vrf_public_key.clone(),
            blocks_produced: v.blocks_produced,
            evaporations_processed: v.evaporations_processed,
            health_score: v.health_score,
            jailed: v.jailed,
            total_slashed: v.total_slashed,
        }
    }
}

impl ValidatorInfoSnapshot {
    pub fn into_validator_info(self) -> crate::validator_set::ValidatorInfo {
        crate::validator_set::ValidatorInfo {
            id: self.id,
            stake: self.stake,
            address: self.address,
            bls_public_key: self.bls_public_key,
            vrf_public_key: self.vrf_public_key,
            blocks_produced: self.blocks_produced,
            evaporations_processed: self.evaporations_processed,
            health_score: self.health_score,
            jailed: self.jailed,
            total_slashed: self.total_slashed,
            bls_pop: None,
            pop_verified: false,
        }
    }
}

impl ConsensusCheckpoint {
    /// Build a checkpoint from the current consensus state.
    pub fn from_consensus(
        height: u64,
        epoch: u64,
        parent_hash: [u8; 32],
        validator_set: &ValidatorSet,
        weak_subjectivity_checkpoints: &[(u64, [u8; 32])],
    ) -> Self {
        Self {
            height,
            epoch,
            parent_hash,
            weak_subjectivity_checkpoints: weak_subjectivity_checkpoints.to_vec(),
            validators: validator_set
                .validators()
                .iter()
                .map(ValidatorInfoSnapshot::from)
                .collect(),
        }
    }

    /// Reconstruct the validator set from the snapshot.
    pub fn restore_validator_set(&self) -> ValidatorSet {
        let validators: Vec<_> = self
            .validators
            .iter()
            .cloned()
            .map(|s| s.into_validator_info())
            .collect();
        ValidatorSet::with_validators(validators)
    }
}

// ─────────────────────── WAL Entry ──────────────────────────────────────

/// Write-ahead log entry — records consensus actions before they happen
/// so the node can avoid double-voting after a crash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalEntry {
    /// Consensus height this entry belongs to.
    pub height: u64,
    /// Consensus round.
    pub round: u32,
    /// What happened.
    pub action: WalAction,
    /// Monotonic sequence number for ordering.
    pub seq: u64,
}

/// Actions recorded in the WAL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WalAction {
    /// We sent a prevote for this hash (or None for nil).
    SentPrevote { block_hash: Option<[u8; 32]> },
    /// We sent a precommit for this hash (or None for nil).
    SentPrecommit { block_hash: Option<[u8; 32]> },
    /// We locked on a block at this round.
    LockedBlock { round: u32, block_hash: [u8; 32] },
}

// ─────────────────────── Store Trait ────────────────────────────────────

/// Trait for consensus state persistence.
pub trait ConsensusStateStore: Send + Sync {
    /// Save a checkpoint (called on every commit).
    fn save_checkpoint(&self, checkpoint: &ConsensusCheckpoint) -> io::Result<()>;

    /// Load the most recent checkpoint (called on startup).
    fn load_checkpoint(&self) -> io::Result<Option<ConsensusCheckpoint>>;

    /// Append a WAL entry (called before voting).
    fn append_wal(&self, entry: &WalEntry) -> io::Result<()>;

    /// Load all WAL entries for a given height (called on recovery).
    fn load_wal(&self, height: u64) -> io::Result<Vec<WalEntry>>;

    /// Clear WAL entries at or below a height (called after commit).
    fn clear_wal(&self, up_to_height: u64) -> io::Result<()>;
}

// ─────────────────────── File-Based Implementation ──────────────────────

/// File-based consensus state store.
///
/// Directory layout:
/// ```text
/// <base_dir>/
///   checkpoint.json          — latest checkpoint (atomic write)
///   wal/
///     <height>_<seq>.json    — WAL entries
/// ```
pub struct FileStateStore {
    base_dir: PathBuf,
    wal_dir: PathBuf,
    /// Monotonic WAL sequence counter.
    seq: std::sync::atomic::AtomicU64,
}

impl FileStateStore {
    /// Create a new file-based state store at the given directory.
    pub fn new(base_dir: impl Into<PathBuf>) -> io::Result<Self> {
        let base_dir = base_dir.into();
        let wal_dir = base_dir.join("wal");
        fs::create_dir_all(&wal_dir)?;

        // Find the highest existing WAL seq to resume from.
        let max_seq = Self::scan_max_seq(&wal_dir);

        Ok(Self {
            base_dir,
            wal_dir,
            seq: std::sync::atomic::AtomicU64::new(max_seq + 1),
        })
    }

    fn checkpoint_path(&self) -> PathBuf {
        self.base_dir.join("checkpoint.json")
    }

    fn scan_max_seq(wal_dir: &Path) -> u64 {
        let mut max = 0u64;
        if let Ok(entries) = fs::read_dir(wal_dir) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    // Format: <height>_<seq>.json
                    if let Some(seq_str) = name.strip_suffix(".json").and_then(|n| n.split('_').nth(1))
                    {
                        if let Ok(seq) = seq_str.parse::<u64>() {
                            max = max.max(seq);
                        }
                    }
                }
            }
        }
        max
    }

    /// Atomic write: serialize → write to temp → fsync → rename over target.
    fn atomic_write(path: &Path, data: &[u8]) -> io::Result<()> {
        let tmp = path.with_extension("tmp");
        {
            let f = fs::File::create(&tmp)?;
            io::Write::write_all(&mut &f, data)?;
            f.sync_all()?;
        }
        fs::rename(&tmp, path)?;
        Ok(())
    }

    fn next_seq(&self) -> u64 {
        self.seq
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }
}

impl ConsensusStateStore for FileStateStore {
    fn save_checkpoint(&self, checkpoint: &ConsensusCheckpoint) -> io::Result<()> {
        let data = serde_json::to_vec_pretty(checkpoint)
            .map_err(io::Error::other)?;
        Self::atomic_write(&self.checkpoint_path(), &data)
    }

    fn load_checkpoint(&self) -> io::Result<Option<ConsensusCheckpoint>> {
        let path = self.checkpoint_path();
        if !path.exists() {
            return Ok(None);
        }
        let data = fs::read(&path)?;
        let checkpoint: ConsensusCheckpoint = serde_json::from_slice(&data)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        Ok(Some(checkpoint))
    }

    fn append_wal(&self, entry: &WalEntry) -> io::Result<()> {
        let seq = self.next_seq();
        let filename = format!("{}_{}.json", entry.height, seq);
        let path = self.wal_dir.join(filename);
        let data = serde_json::to_vec(entry)
            .map_err(io::Error::other)?;
        // fsync WAL entries so they survive power loss.
        let f = fs::File::create(&path)?;
        io::Write::write_all(&mut &f, &data)?;
        f.sync_all()?;
        Ok(())
    }

    fn load_wal(&self, height: u64) -> io::Result<Vec<WalEntry>> {
        let mut entries = Vec::new();
        let prefix = format!("{}_", height);

        if let Ok(dir) = fs::read_dir(&self.wal_dir) {
            for entry in dir.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    if name.starts_with(&prefix) && name.ends_with(".json") {
                        let data = fs::read(entry.path())?;
                        // Skip corrupted entries (partial writes from crash).
                        if let Ok(wal_entry) = serde_json::from_slice::<WalEntry>(&data) {
                            entries.push(wal_entry);
                        }
                    }
                }
            }
        }

        entries.sort_by_key(|e| e.seq);
        Ok(entries)
    }

    fn clear_wal(&self, up_to_height: u64) -> io::Result<()> {
        if let Ok(dir) = fs::read_dir(&self.wal_dir) {
            for entry in dir.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    if let Some(height_str) = name.split('_').next() {
                        if let Ok(h) = height_str.parse::<u64>() {
                            if h <= up_to_height {
                                let _ = fs::remove_file(entry.path());
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

// ─────────────────────── In-Memory Implementation (for tests) ───────────

/// In-memory state store for testing.
#[derive(Default)]
pub struct InMemoryStateStore {
    checkpoint: std::sync::Mutex<Option<ConsensusCheckpoint>>,
    wal: std::sync::Mutex<Vec<WalEntry>>,
}

impl InMemoryStateStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl ConsensusStateStore for InMemoryStateStore {
    fn save_checkpoint(&self, checkpoint: &ConsensusCheckpoint) -> io::Result<()> {
        *self.checkpoint.lock().unwrap() = Some(checkpoint.clone());
        Ok(())
    }

    fn load_checkpoint(&self) -> io::Result<Option<ConsensusCheckpoint>> {
        Ok(self.checkpoint.lock().unwrap().clone())
    }

    fn append_wal(&self, entry: &WalEntry) -> io::Result<()> {
        self.wal.lock().unwrap().push(entry.clone());
        Ok(())
    }

    fn load_wal(&self, height: u64) -> io::Result<Vec<WalEntry>> {
        let entries: Vec<_> = self
            .wal
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.height == height)
            .cloned()
            .collect();
        Ok(entries)
    }

    fn clear_wal(&self, up_to_height: u64) -> io::Result<()> {
        self.wal
            .lock()
            .unwrap()
            .retain(|e| e.height > up_to_height);
        Ok(())
    }
}

// ─────────────────────────── Tests ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validator_set::ValidatorInfo;
    use tempfile::TempDir;

    fn make_checkpoint(height: u64) -> ConsensusCheckpoint {
        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::new(1, 1000, [1u8; 32]));
        vs.add_validator(ValidatorInfo::new(2, 2000, [2u8; 32]));

        ConsensusCheckpoint::from_consensus(
            height,
            height,
            [height as u8; 32],
            &vs,
            &[(100, [0xAA; 32])],
        )
    }

    // ─── InMemoryStateStore ─────────────────────────────────────────

    #[test]
    fn test_in_memory_checkpoint_roundtrip() {
        let store = InMemoryStateStore::new();
        assert!(store.load_checkpoint().unwrap().is_none());

        let cp = make_checkpoint(42);
        store.save_checkpoint(&cp).unwrap();

        let loaded = store.load_checkpoint().unwrap().unwrap();
        assert_eq!(loaded.height, 42);
        assert_eq!(loaded.epoch, 42);
        assert_eq!(loaded.parent_hash, [42u8; 32]);
        assert_eq!(loaded.validators.len(), 2);
        assert_eq!(loaded.weak_subjectivity_checkpoints.len(), 1);
    }

    #[test]
    fn test_in_memory_wal_operations() {
        let store = InMemoryStateStore::new();

        let e1 = WalEntry {
            height: 10,
            round: 0,
            action: WalAction::SentPrevote {
                block_hash: Some([1u8; 32]),
            },
            seq: 1,
        };
        let e2 = WalEntry {
            height: 10,
            round: 0,
            action: WalAction::SentPrecommit {
                block_hash: Some([1u8; 32]),
            },
            seq: 2,
        };
        let e3 = WalEntry {
            height: 11,
            round: 0,
            action: WalAction::SentPrevote {
                block_hash: None,
            },
            seq: 3,
        };

        store.append_wal(&e1).unwrap();
        store.append_wal(&e2).unwrap();
        store.append_wal(&e3).unwrap();

        let h10 = store.load_wal(10).unwrap();
        assert_eq!(h10.len(), 2);

        let h11 = store.load_wal(11).unwrap();
        assert_eq!(h11.len(), 1);

        // Clear WAL up to height 10
        store.clear_wal(10).unwrap();
        assert_eq!(store.load_wal(10).unwrap().len(), 0);
        assert_eq!(store.load_wal(11).unwrap().len(), 1);
    }

    #[test]
    fn test_checkpoint_restores_validator_set() {
        let cp = make_checkpoint(50);
        let vs = cp.restore_validator_set();
        assert_eq!(vs.len(), 2);
        assert_eq!(vs.get(1).unwrap().stake, 1000);
        assert_eq!(vs.get(2).unwrap().stake, 2000);
    }

    // ─── FileStateStore ─────────────────────────────────────────────

    #[test]
    fn test_file_checkpoint_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let store = FileStateStore::new(tmp.path()).unwrap();

        assert!(store.load_checkpoint().unwrap().is_none());

        let cp = make_checkpoint(99);
        store.save_checkpoint(&cp).unwrap();

        let loaded = store.load_checkpoint().unwrap().unwrap();
        assert_eq!(loaded.height, 99);
        assert_eq!(loaded.validators.len(), 2);
        assert_eq!(loaded.weak_subjectivity_checkpoints.len(), 1);
    }

    #[test]
    fn test_file_wal_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let store = FileStateStore::new(tmp.path()).unwrap();

        let e1 = WalEntry {
            height: 5,
            round: 0,
            action: WalAction::SentPrevote {
                block_hash: Some([0xBB; 32]),
            },
            seq: 0,
        };
        let e2 = WalEntry {
            height: 5,
            round: 1,
            action: WalAction::LockedBlock {
                round: 0,
                block_hash: [0xBB; 32],
            },
            seq: 0,
        };

        store.append_wal(&e1).unwrap();
        store.append_wal(&e2).unwrap();

        let loaded = store.load_wal(5).unwrap();
        assert_eq!(loaded.len(), 2);
    }

    #[test]
    fn test_file_wal_clear() {
        let tmp = TempDir::new().unwrap();
        let store = FileStateStore::new(tmp.path()).unwrap();

        for h in 1..=5 {
            store
                .append_wal(&WalEntry {
                    height: h,
                    round: 0,
                    action: WalAction::SentPrevote { block_hash: None },
                    seq: 0,
                })
                .unwrap();
        }

        store.clear_wal(3).unwrap();

        assert_eq!(store.load_wal(1).unwrap().len(), 0);
        assert_eq!(store.load_wal(2).unwrap().len(), 0);
        assert_eq!(store.load_wal(3).unwrap().len(), 0);
        assert_eq!(store.load_wal(4).unwrap().len(), 1);
        assert_eq!(store.load_wal(5).unwrap().len(), 1);
    }

    #[test]
    fn test_file_checkpoint_overwrite() {
        let tmp = TempDir::new().unwrap();
        let store = FileStateStore::new(tmp.path()).unwrap();

        store.save_checkpoint(&make_checkpoint(1)).unwrap();
        store.save_checkpoint(&make_checkpoint(2)).unwrap();

        let loaded = store.load_checkpoint().unwrap().unwrap();
        assert_eq!(loaded.height, 2, "Should keep only latest checkpoint");
    }

    #[test]
    fn test_atomic_write_no_partial() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.json");

        // Write data atomically
        FileStateStore::atomic_write(&path, b"{\"ok\":true}").unwrap();
        assert!(path.exists());

        // Temp file should be cleaned up
        assert!(!path.with_extension("tmp").exists());
    }

    #[test]
    fn test_file_store_survives_restart() {
        let tmp = TempDir::new().unwrap();

        // Simulate first run
        {
            let store = FileStateStore::new(tmp.path()).unwrap();
            store.save_checkpoint(&make_checkpoint(100)).unwrap();
            store
                .append_wal(&WalEntry {
                    height: 101,
                    round: 0,
                    action: WalAction::SentPrevote {
                        block_hash: Some([0xFF; 32]),
                    },
                    seq: 0,
                })
                .unwrap();
        }

        // Simulate restart — new store instance, same directory
        {
            let store = FileStateStore::new(tmp.path()).unwrap();
            let cp = store.load_checkpoint().unwrap().unwrap();
            assert_eq!(cp.height, 100);

            let wal = store.load_wal(101).unwrap();
            assert_eq!(wal.len(), 1);
            match &wal[0].action {
                WalAction::SentPrevote { block_hash } => {
                    assert_eq!(*block_hash, Some([0xFF; 32]));
                }
                _ => panic!("Expected SentPrevote"),
            }
        }
    }
}
