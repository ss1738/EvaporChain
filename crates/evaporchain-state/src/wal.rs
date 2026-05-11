//! Write-Ahead Log for crash-safe state transitions.
//!
//! Before applying a block's mutations to the state DB, the node writes
//! a WAL entry containing the block height, pre-state root, and serialized
//! mutations. On crash recovery the node reads the WAL:
//!   - If a COMMIT marker exists for the entry → state was fully applied, safe.
//!   - If no COMMIT marker → discard partial state, replay from last committed height.
//!
//! File format (append-only):
//!   [8B height][32B pre_state_root][4B payload_len][payload][32B checksum][1B marker]
//! Where marker = 0x00 for BEGIN, 0x01 for COMMIT.

use blake3;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const WAL_MAGIC: &[u8; 8] = b"EVAPWAL\x01";
const MARKER_BEGIN: u8 = 0x00;
const MARKER_COMMIT: u8 = 0x01;
const HEADER_SIZE: usize = 8; // magic

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum WalMutation {
    PutAccount { address: [u8; 20], data: Vec<u8> },
    DeleteAccount { address: [u8; 20] },
    PutObject { id: [u8; 20], data: Vec<u8> },
    DeleteObject { id: [u8; 20] },
    PutGhost { id: [u8; 20], data: Vec<u8> },
    RemoveGhost { id: [u8; 20] },
    PutStake { validator_id: u64, data: Vec<u8> },
    RemoveStake { validator_id: u64 },
    SpendNullifier { nullifier: [u8; 32] },
    SetNoteTreeRoot { root: [u8; 32] },
    SetShieldedPoolBalance { balance: u64 },
    SetNoteCount { count: u64 },
    PutProposal { proposal_id: u64, data: Vec<u8> },
    PutGovernanceParam { key: String, value: String },
}

#[derive(Debug, Clone)]
pub struct WalEntry {
    pub height: u64,
    pub pre_state_root: [u8; 32],
    pub mutations: Vec<WalMutation>,
    pub committed: bool,
}

pub struct WriteAheadLog {
    _path: PathBuf,
    file: File,
    last_committed_height: Option<u64>,
}

impl WriteAheadLog {
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let exists = path.exists();
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)?;

        if !exists || file.metadata()?.len() == 0 {
            file.write_all(WAL_MAGIC)?;
            file.sync_all()?;
        } else {
            let mut magic = [0u8; HEADER_SIZE];
            file.read_exact(&mut magic)?;
            if &magic != WAL_MAGIC {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "WAL file has invalid magic bytes",
                ));
            }
        }

        let last_committed_height = Self::scan_last_committed(&mut file)?;
        file.seek(SeekFrom::End(0))?;

        Ok(Self {
            _path: path,
            file,
            last_committed_height,
        })
    }

    pub fn begin_block(
        &mut self,
        height: u64,
        pre_state_root: [u8; 32],
        mutations: &[WalMutation],
    ) -> io::Result<()> {
        let payload = bincode::serialize(mutations)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let checksum = blake3::hash(&payload);

        self.file.write_all(&height.to_le_bytes())?;
        self.file.write_all(&pre_state_root)?;
        let len = payload.len() as u32;
        self.file.write_all(&len.to_le_bytes())?;
        self.file.write_all(&payload)?;
        self.file.write_all(checksum.as_bytes())?;
        self.file.write_all(&[MARKER_BEGIN])?;
        self.file.sync_all()?;
        Ok(())
    }

    pub fn commit_block(&mut self, height: u64) -> io::Result<()> {
        let payload = bincode::serialize(&Vec::<WalMutation>::new())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let checksum = blake3::hash(&payload);

        self.file.write_all(&height.to_le_bytes())?;
        self.file.write_all(&[0u8; 32])?; // no pre_state_root for commit marker
        let len = payload.len() as u32;
        self.file.write_all(&len.to_le_bytes())?;
        self.file.write_all(&payload)?;
        self.file.write_all(checksum.as_bytes())?;
        self.file.write_all(&[MARKER_COMMIT])?;
        self.file.sync_all()?;
        self.last_committed_height = Some(height);
        Ok(())
    }

    pub fn last_committed_height(&self) -> Option<u64> {
        self.last_committed_height
    }

    /// Recover uncommitted entries (blocks that were begun but not committed).
    pub fn recover_uncommitted(&mut self) -> io::Result<Vec<WalEntry>> {
        self.file.seek(SeekFrom::Start(HEADER_SIZE as u64))?;
        let entries = Self::read_all_entries(&mut self.file)?;

        let mut committed_heights = std::collections::HashSet::new();
        for entry in &entries {
            if entry.committed {
                committed_heights.insert(entry.height);
            }
        }

        Ok(entries
            .into_iter()
            .filter(|e| !e.committed && !committed_heights.contains(&e.height))
            .collect())
    }

    /// Truncate the WAL after recovery, keeping only the magic header.
    /// Call this after state has been verified consistent.
    pub fn truncate_after_recovery(&mut self) -> io::Result<()> {
        self.file.set_len(HEADER_SIZE as u64)?;
        self.file.seek(SeekFrom::End(0))?;
        self.file.sync_all()?;
        Ok(())
    }

    /// Compact: remove all entries for heights <= committed_up_to.
    pub fn compact(&mut self, committed_up_to: u64) -> io::Result<()> {
        self.file.seek(SeekFrom::Start(HEADER_SIZE as u64))?;
        let entries = Self::read_all_entries(&mut self.file)?;

        let remaining: Vec<&WalEntry> = entries
            .iter()
            .filter(|e| e.height > committed_up_to)
            .collect();

        self.file.seek(SeekFrom::Start(0))?;
        self.file.set_len(0)?;
        self.file.write_all(WAL_MAGIC)?;

        for entry in remaining {
            let payload = bincode::serialize(&entry.mutations)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            let checksum = blake3::hash(&payload);
            let marker = if entry.committed {
                MARKER_COMMIT
            } else {
                MARKER_BEGIN
            };

            self.file.write_all(&entry.height.to_le_bytes())?;
            self.file.write_all(&entry.pre_state_root)?;
            let len = payload.len() as u32;
            self.file.write_all(&len.to_le_bytes())?;
            self.file.write_all(&payload)?;
            self.file.write_all(checksum.as_bytes())?;
            self.file.write_all(&[marker])?;
        }

        self.file.sync_all()?;
        Ok(())
    }

    fn scan_last_committed(file: &mut File) -> io::Result<Option<u64>> {
        file.seek(SeekFrom::Start(HEADER_SIZE as u64))?;
        let entries = Self::read_all_entries(file)?;
        let last = entries.iter().rev().find(|e| e.committed).map(|e| e.height);
        Ok(last)
    }

    fn read_all_entries(file: &mut File) -> io::Result<Vec<WalEntry>> {
        let mut entries = Vec::new();
        let file_len = file.metadata()?.len();

        loop {
            let pos = file.stream_position()?;
            if pos >= file_len {
                break;
            }
            // Minimum entry: 8 + 32 + 4 + 0 + 32 + 1 = 77 bytes
            if file_len - pos < 77 {
                break;
            }

            let mut height_buf = [0u8; 8];
            if file.read_exact(&mut height_buf).is_err() {
                break;
            }
            let height = u64::from_le_bytes(height_buf);

            let mut root_buf = [0u8; 32];
            if file.read_exact(&mut root_buf).is_err() {
                break;
            }

            let mut len_buf = [0u8; 4];
            if file.read_exact(&mut len_buf).is_err() {
                break;
            }
            let payload_len = u32::from_le_bytes(len_buf) as usize;

            if payload_len > 64 * 1024 * 1024 {
                break; // sanity cap: 64MB per entry
            }

            let mut payload = vec![0u8; payload_len];
            if file.read_exact(&mut payload).is_err() {
                break;
            }

            let mut checksum_buf = [0u8; 32];
            if file.read_exact(&mut checksum_buf).is_err() {
                break;
            }

            let computed = blake3::hash(&payload);
            if computed.as_bytes() != &checksum_buf {
                break; // corrupted entry — stop here
            }

            let mut marker = [0u8; 1];
            if file.read_exact(&mut marker).is_err() {
                break;
            }

            let committed = marker[0] == MARKER_COMMIT;
            let mutations: Vec<WalMutation> = bincode::deserialize(&payload)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

            entries.push(WalEntry {
                height,
                pre_state_root: root_buf,
                mutations,
                committed,
            });
        }

        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn tmp_wal() -> (WriteAheadLog, NamedTempFile) {
        let f = NamedTempFile::new().unwrap();
        let wal = WriteAheadLog::open(f.path()).unwrap();
        (wal, f)
    }

    #[test]
    fn test_open_creates_valid_wal() {
        let (wal, _f) = tmp_wal();
        assert_eq!(wal.last_committed_height(), None);
    }

    #[test]
    fn test_begin_and_commit() {
        let (mut wal, _f) = tmp_wal();
        let mutations = vec![
            WalMutation::PutAccount {
                address: [1u8; 20],
                data: vec![10, 20, 30],
            },
            WalMutation::PutObject {
                id: [2u8; 20],
                data: vec![40, 50],
            },
        ];
        wal.begin_block(1, [0xAA; 32], &mutations).unwrap();
        assert_eq!(wal.last_committed_height(), None);

        wal.commit_block(1).unwrap();
        assert_eq!(wal.last_committed_height(), Some(1));
    }

    #[test]
    fn test_recover_uncommitted_empty() {
        let (mut wal, _f) = tmp_wal();
        let uncommitted = wal.recover_uncommitted().unwrap();
        assert!(uncommitted.is_empty());
    }

    #[test]
    fn test_recover_uncommitted_after_crash() {
        let (mut wal, f) = tmp_wal();
        let mutations = vec![WalMutation::PutAccount {
            address: [3u8; 20],
            data: vec![1, 2, 3],
        }];
        wal.begin_block(5, [0xBB; 32], &mutations).unwrap();
        // Simulate crash: no commit_block(5)

        // Reopen
        drop(wal);
        let mut wal2 = WriteAheadLog::open(f.path()).unwrap();
        let uncommitted = wal2.recover_uncommitted().unwrap();
        assert_eq!(uncommitted.len(), 1);
        assert_eq!(uncommitted[0].height, 5);
        assert_eq!(uncommitted[0].pre_state_root, [0xBB; 32]);
        assert_eq!(uncommitted[0].mutations, mutations);
        assert!(!uncommitted[0].committed);
    }

    #[test]
    fn test_committed_block_not_in_uncommitted() {
        let (mut wal, f) = tmp_wal();
        let m1 = vec![WalMutation::PutAccount {
            address: [1u8; 20],
            data: vec![1],
        }];
        let m2 = vec![WalMutation::PutAccount {
            address: [2u8; 20],
            data: vec![2],
        }];

        wal.begin_block(1, [0xAA; 32], &m1).unwrap();
        wal.commit_block(1).unwrap();
        wal.begin_block(2, [0xBB; 32], &m2).unwrap();
        // Block 2 not committed

        drop(wal);
        let mut wal2 = WriteAheadLog::open(f.path()).unwrap();
        assert_eq!(wal2.last_committed_height(), Some(1));
        let uncommitted = wal2.recover_uncommitted().unwrap();
        assert_eq!(uncommitted.len(), 1);
        assert_eq!(uncommitted[0].height, 2);
    }

    #[test]
    fn test_truncate_after_recovery() {
        let (mut wal, f) = tmp_wal();
        let m = vec![WalMutation::SpendNullifier {
            nullifier: [0xFF; 32],
        }];
        wal.begin_block(10, [0; 32], &m).unwrap();
        wal.commit_block(10).unwrap();
        wal.truncate_after_recovery().unwrap();

        drop(wal);
        let mut wal2 = WriteAheadLog::open(f.path()).unwrap();
        assert_eq!(wal2.last_committed_height(), None);
        assert!(wal2.recover_uncommitted().unwrap().is_empty());
    }

    #[test]
    fn test_compact_removes_old_entries() {
        let (mut wal, f) = tmp_wal();
        for h in 1..=5 {
            let m = vec![WalMutation::SetNoteCount { count: h }];
            wal.begin_block(h, [0; 32], &m).unwrap();
            wal.commit_block(h).unwrap();
        }
        wal.compact(3).unwrap();

        drop(wal);
        let mut wal2 = WriteAheadLog::open(f.path()).unwrap();
        let uncommitted = wal2.recover_uncommitted().unwrap();
        assert!(uncommitted.is_empty());
        // Heights 4 and 5 should survive compaction
        assert_eq!(wal2.last_committed_height(), Some(5));
    }

    #[test]
    fn test_corrupted_entry_stops_read() {
        let (mut wal, f) = tmp_wal();
        let m = vec![WalMutation::PutAccount {
            address: [9u8; 20],
            data: vec![1],
        }];
        wal.begin_block(1, [0; 32], &m).unwrap();
        wal.commit_block(1).unwrap();

        // Append garbage
        drop(wal);
        let mut raw = OpenOptions::new().append(true).open(f.path()).unwrap();
        raw.write_all(&[0xFF; 100]).unwrap();
        drop(raw);

        let mut wal2 = WriteAheadLog::open(f.path()).unwrap();
        assert_eq!(wal2.last_committed_height(), Some(1));
        assert!(wal2.recover_uncommitted().unwrap().is_empty());
    }

    #[test]
    fn test_multiple_mutations_roundtrip() {
        let (mut wal, f) = tmp_wal();
        let mutations = vec![
            WalMutation::PutAccount {
                address: [1u8; 20],
                data: vec![10],
            },
            WalMutation::DeleteAccount { address: [2u8; 20] },
            WalMutation::PutObject {
                id: [3u8; 20],
                data: vec![30],
            },
            WalMutation::DeleteObject { id: [4u8; 20] },
            WalMutation::PutGhost {
                id: [5u8; 20],
                data: vec![50],
            },
            WalMutation::RemoveGhost { id: [6u8; 20] },
            WalMutation::PutStake {
                validator_id: 7,
                data: vec![70],
            },
            WalMutation::RemoveStake { validator_id: 8 },
            WalMutation::SpendNullifier {
                nullifier: [9u8; 32],
            },
            WalMutation::SetNoteTreeRoot { root: [10u8; 32] },
            WalMutation::SetShieldedPoolBalance { balance: 1000 },
            WalMutation::SetNoteCount { count: 42 },
            WalMutation::PutProposal {
                proposal_id: 1,
                data: vec![1, 2],
            },
            WalMutation::PutGovernanceParam {
                key: "max_gas".into(),
                value: "10000000".into(),
            },
        ];
        wal.begin_block(99, [0xCC; 32], &mutations).unwrap();

        drop(wal);
        let mut wal2 = WriteAheadLog::open(f.path()).unwrap();
        let uncommitted = wal2.recover_uncommitted().unwrap();
        assert_eq!(uncommitted.len(), 1);
        assert_eq!(uncommitted[0].mutations.len(), 14);
        assert_eq!(uncommitted[0].mutations, mutations);
    }

    #[test]
    fn test_reopen_preserves_state() {
        let (mut wal, f) = tmp_wal();
        let m = vec![WalMutation::SetNoteCount { count: 1 }];
        wal.begin_block(1, [0; 32], &m).unwrap();
        wal.commit_block(1).unwrap();

        let m2 = vec![WalMutation::SetNoteCount { count: 2 }];
        wal.begin_block(2, [0; 32], &m2).unwrap();
        wal.commit_block(2).unwrap();

        drop(wal);
        let wal2 = WriteAheadLog::open(f.path()).unwrap();
        assert_eq!(wal2.last_committed_height(), Some(2));
    }

    // ─── T1.20 coverage push for wal.rs ─────────────────────────────

    /// WAL_MAGIC check: opening a file with wrong magic bytes
    /// returns an io::Error with InvalidData kind.
    #[test]
    fn t1_20_wal_open_rejects_invalid_magic() {
        let f = NamedTempFile::new().unwrap();
        // Pre-write wrong magic bytes so open() reads them as
        // existing-file content.
        {
            let mut h = OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(f.path())
                .unwrap();
            h.write_all(b"NOTAWAL!").unwrap(); // 8 bytes, wrong magic
        }
        let result = WriteAheadLog::open(f.path());
        match result {
            Err(e) => {
                assert_eq!(e.kind(), io::ErrorKind::InvalidData);
                assert!(format!("{e}").contains("invalid magic"));
            }
            Ok(_) => panic!("expected InvalidData error"),
        }
    }

    /// Truncated entry — file ends mid-record after a valid commit.
    /// scan_last_committed must return the last valid height and
    /// stop at the truncation boundary.
    #[test]
    fn t1_20_wal_truncated_mid_entry_stops_scan_gracefully() {
        let f = NamedTempFile::new().unwrap();
        {
            let mut wal = WriteAheadLog::open(f.path()).unwrap();
            let m = vec![WalMutation::PutAccount {
                address: [1u8; 20],
                data: vec![0xAA],
            }];
            wal.begin_block(7, [0; 32], &m).unwrap();
            wal.commit_block(7).unwrap();
        }

        // Truncate the file mid-way through the next would-be entry
        // by writing < 77 bytes of garbage.
        {
            let mut h = OpenOptions::new()
                .append(true)
                .open(f.path())
                .unwrap();
            h.write_all(&[0u8; 30]).unwrap(); // < 77 bytes
        }

        // Re-open and scan: must see commit at 7 and stop cleanly.
        let wal2 = WriteAheadLog::open(f.path()).unwrap();
        assert_eq!(wal2.last_committed_height(), Some(7));
    }

    /// Oversized payload sentinel — entry claiming payload_len >
    /// 64 MiB is treated as corruption (the sanity cap).
    #[test]
    fn t1_20_wal_oversized_payload_sentinel_stops_scan() {
        let f = NamedTempFile::new().unwrap();
        {
            let mut wal = WriteAheadLog::open(f.path()).unwrap();
            let m = vec![WalMutation::PutAccount {
                address: [2u8; 20],
                data: vec![0xBB],
            }];
            wal.begin_block(3, [0; 32], &m).unwrap();
            wal.commit_block(3).unwrap();
        }
        // Hand-craft an entry with payload_len = u32::MAX (well over
        // the 64MB cap). Format: 8 bytes height + 32 bytes root +
        // 4 bytes len. Pad with enough zeros so file_len - pos >= 77.
        {
            let mut h = OpenOptions::new()
                .append(true)
                .open(f.path())
                .unwrap();
            let mut buf = Vec::new();
            buf.extend_from_slice(&100u64.to_le_bytes()); // height
            buf.extend_from_slice(&[0xCC; 32]); // root
            buf.extend_from_slice(&u32::MAX.to_le_bytes()); // payload_len
            buf.extend_from_slice(&[0u8; 80]); // padding
            h.write_all(&buf).unwrap();
        }
        let wal2 = WriteAheadLog::open(f.path()).unwrap();
        // Scan should hit the cap-check break and stop at h=3.
        assert_eq!(wal2.last_committed_height(), Some(3));
    }
}
