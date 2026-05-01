//! RocksDB-backed state database with write-through cache.
//!
//! Maintains in-memory HashMaps for zero-overhead reads while persisting
//! every mutation to RocksDB. On startup, all data is loaded from disk
//! into the cache, so the node resumes exactly where it left off.

use crate::db::{
    build_energy_trie, trie_key_for_account, trie_key_for_object, trie_value_for_account,
    trie_value_for_object, StateDB,
};
use evaporchain_crypto::{EnergyVerkleTrie, TrieHealth};
use evaporchain_types::{
    Account, AccountAddress, DelegationRecord, GhostRecord, ObjectId, StakeRecord, StateObject,
};
use rocksdb::{ColumnFamily, ColumnFamilyDescriptor, Options, WriteBatch, DB};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

const CF_OBJECTS: &str = "objects";
const CF_GHOSTS: &str = "ghosts";
const CF_ACCOUNTS: &str = "accounts";
const CF_TRIE: &str = "trie";
const CF_NULLIFIERS: &str = "nullifiers";
/// Append-only note-commitment archive. Key = u64 leaf index (big-endian
/// for sort order), value = 32-byte commitment. Required so the in-memory
/// note tree can be rebuilt at startup without replaying the chain from
/// genesis. Closes punch-list 1a.
const CF_NOTE_COMMITMENTS: &str = "note_commitments";
/// Sentinel autonomic-governance: bounded parameters keyed by u32 id
/// (big-endian for sorted iteration). Per INVENTION_STACK.md §A2.5.
const CF_SENTINEL_PARAMS: &str = "sentinel_params";
/// Sentinel votes: full vote slate per parameter id, replaced atomically
/// (one-vote-per-validator semantics handled at write site).
const CF_SENTINEL_VOTES: &str = "sentinel_votes";
/// Validator stake records keyed by validator_id (big-endian u64).
const CF_STAKES: &str = "stakes";
/// Delegation records keyed by (delegator_address[32] ++ validator_id_be[8]).
const CF_DELEGATIONS: &str = "delegations";
const TRIE_SNAPSHOT_KEY: &[u8] = b"__energy_verkle_trie__";
const PRIVACY_NOTE_ROOT_KEY: &[u8] = b"__note_tree_root__";
const PRIVACY_POOL_BALANCE_KEY: &[u8] = b"__shielded_pool_balance__";
const PRIVACY_NOTE_COUNT_KEY: &[u8] = b"__note_count__";
const LAST_RENT_EPOCH_KEY: &[u8] = b"__last_rent_epoch__";

/// Halt the node when a persistence operation fails. Logs the operation +
/// underlying error, then exits with status 2 so the operator can diagnose.
///
/// Why this exists: previously the persistence path used `.expect("write X
/// to RocksDB")`, which panics with a generic message, no structured log,
/// and a 101 exit status. For an L1 the difference between "node panicked
/// at line 338" and "FATAL persistence failure: write_object returned IO
/// error: disk full" is substantial — both for diagnosis and for
/// distinguishing programmer-error panics from genuine I/O failures.
///
/// Why exit instead of `Result` propagation: the public `StateDB` trait
/// methods (`put_object`, `put_account`, etc.) return unit. Adapting them
/// to `Result<(), PersistError>` would cascade through every caller in
/// the workspace. Logged-exit halts the chain cleanly without that
/// cascade and produces an actionable log line; on restart, the node
/// reloads from disk and the in-flight block is replayed by consensus.
/// Closes the gap from `audit/end_to_end_audit_2026_04_27.md` §3.
fn fatal_persistence_error(op: &'static str, e: impl std::fmt::Display) -> ! {
    tracing::error!(
        operation = op,
        error = %e,
        "FATAL: persistence failure — node halting to prevent state corruption",
    );
    // Give the tracing subscriber a moment to flush before exit.
    std::thread::sleep(std::time::Duration::from_millis(100));
    std::process::exit(2);
}

/// Tracks in-memory changes during a batch for correct rollback.
struct BatchUndoLog {
    objects: Vec<(ObjectId, Option<StateObject>)>,
    accounts: Vec<(AccountAddress, Option<Account>)>,
    ghosts: Vec<(ObjectId, Option<GhostRecord>)>,
    dirty_objects: HashSet<ObjectId>,
    dirty_accounts: HashSet<AccountAddress>,
    trie_snapshot: Vec<u8>,
    // Privacy state snapshot for rollback
    note_tree_root: [u8; 32],
    nullifiers_snapshot: HashSet<[u8; 32]>,
    shielded_pool_balance: u64,
    note_count: u64,
    note_commitments_snapshot: BTreeMap<u64, [u8; 32]>,
}

/// RocksDB-backed state database with in-memory write-through cache.
pub struct RocksDBStateDB {
    db: DB,
    objects: HashMap<ObjectId, StateObject>,
    ghosts: HashMap<ObjectId, GhostRecord>,
    accounts: HashMap<AccountAddress, Account>,
    trie: EnergyVerkleTrie,
    dirty_objects: HashSet<ObjectId>,
    dirty_accounts: HashSet<AccountAddress>,
    // Privacy layer state (write-through to RocksDB)
    note_tree_root: [u8; 32],
    spent_nullifiers: std::collections::HashSet<[u8; 32]>,
    shielded_pool_balance: u64,
    note_count: u64,
    /// In-memory cache of persisted note commitments by leaf index. BTreeMap
    /// preserves leaf-order iteration so PrivacyExecutor can rebuild the
    /// note tree deterministically at startup.
    note_commitments: BTreeMap<u64, [u8; 32]>,
    /// Last epoch at which `collect_storage_rent` ran. Persisted across
    /// restarts so rent cadence is exactly per-epoch (not per-block).
    /// Closes punch-list 6.
    last_rent_epoch: u64,
    /// Sentinel governable parameters, write-through cached (§A2.5).
    sentinel_params: BTreeMap<u32, evaporchain_sentinel::BoundedParameter>,
    /// Sentinel votes per parameter id, write-through cached.
    sentinel_votes: BTreeMap<u32, Vec<evaporchain_sentinel::Vote>>,
    /// Validator stake records, keyed by validator_id.
    stakes: HashMap<u64, StakeRecord>,
    /// Delegation records, keyed by delegation_key(delegator, validator_id).
    delegations: HashMap<[u8; 40], DelegationRecord>,
    // Batch mode: buffer writes for atomic commit (Mutex for Sync)
    pending_batch: std::sync::Mutex<Option<WriteBatch>>,
    // Undo log for reverting in-memory state on rollback
    batch_undo: Option<BatchUndoLog>,
}

impl RocksDBStateDB {
    /// Open or create a RocksDB database at the given path.
    /// Loads all existing data into memory on startup.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);

        let cf_descriptors = vec![
            ColumnFamilyDescriptor::new(CF_OBJECTS, Options::default()),
            ColumnFamilyDescriptor::new(CF_GHOSTS, Options::default()),
            ColumnFamilyDescriptor::new(CF_ACCOUNTS, Options::default()),
            ColumnFamilyDescriptor::new(CF_TRIE, Options::default()),
            ColumnFamilyDescriptor::new(CF_NULLIFIERS, Options::default()),
            ColumnFamilyDescriptor::new(CF_NOTE_COMMITMENTS, Options::default()),
            ColumnFamilyDescriptor::new(CF_SENTINEL_PARAMS, Options::default()),
            ColumnFamilyDescriptor::new(CF_SENTINEL_VOTES, Options::default()),
            ColumnFamilyDescriptor::new(CF_STAKES, Options::default()),
            ColumnFamilyDescriptor::new(CF_DELEGATIONS, Options::default()),
        ];

        let db = DB::open_cf_descriptors(&opts, path, cf_descriptors)
            .map_err(|e| format!("Failed to open RocksDB: {}", e))?;

        // Load all data from disk into memory
        let mut objects = HashMap::new();
        let mut ghosts = HashMap::new();
        let mut accounts = HashMap::new();

        // Load objects
        let cf_obj = db
            .cf_handle(CF_OBJECTS)
            .ok_or_else(|| format!("missing column family: {CF_OBJECTS}"))?;
        let iter = db.iterator_cf(cf_obj, rocksdb::IteratorMode::Start);
        for item in iter {
            let (key, value) = item.map_err(|e| format!("RocksDB iterator error: {}", e))?;
            if key.len() == 32 {
                match bincode::deserialize::<StateObject>(&value) {
                    Ok(obj) => {
                        let mut id = [0u8; 32];
                        id.copy_from_slice(&key);
                        objects.insert(id, obj);
                    }
                    Err(e) => {
                        eprintln!("  Warning: skipping corrupt object record: {}", e);
                    }
                }
            }
        }

        // Load ghosts (with fallback for legacy records without mmr_position)
        let cf_ghost = db
            .cf_handle(CF_GHOSTS)
            .ok_or_else(|| format!("missing column family: {CF_GHOSTS}"))?;
        let mut ghost_migrated = 0u32;
        let iter = db.iterator_cf(cf_ghost, rocksdb::IteratorMode::Start);
        for item in iter {
            let (key, value) = item.map_err(|e| format!("RocksDB iterator error: {}", e))?;
            if key.len() == 32 {
                let mut id = [0u8; 32];
                id.copy_from_slice(&key);

                // Try current format first
                if let Ok(ghost) = bincode::deserialize::<GhostRecord>(&value) {
                    ghosts.insert(id, ghost);
                } else if let Ok(ghost) = deserialize_legacy_ghost(&value, &id) {
                    // Legacy format recovered — will be re-persisted below
                    ghosts.insert(id, ghost);
                    ghost_migrated += 1;
                } else {
                    eprintln!(
                        "  Warning: skipping unrecoverable ghost record {}",
                        hex::encode(&id[..4])
                    );
                }
            }
        }
        // Re-persist migrated ghosts in the current format and compact
        if ghost_migrated > 0 {
            let cf_g = db
                .cf_handle(CF_GHOSTS)
                .ok_or_else(|| format!("missing column family: {CF_GHOSTS}"))?;
            for ghost in ghosts.values() {
                let val = match bincode::serialize(ghost) {
                    Ok(v) => v,
                    Err(e) => fatal_persistence_error("migrate_serialize_ghost", e),
                };
                if let Err(e) = db.put_cf(cf_g, ghost.object_id, val) {
                    fatal_persistence_error("migrate_ghost_to_rocksdb", e);
                }
            }
            // Force compaction so old SST entries are replaced
            db.compact_range_cf(cf_g, None::<&[u8]>, None::<&[u8]>);
            eprintln!("  Migrated and compacted {} ghost records", ghost_migrated);
        }

        // Load accounts
        let cf_acct = db
            .cf_handle(CF_ACCOUNTS)
            .ok_or_else(|| format!("missing column family: {CF_ACCOUNTS}"))?;
        let iter = db.iterator_cf(cf_acct, rocksdb::IteratorMode::Start);
        for item in iter {
            let (key, value) = item.map_err(|e| format!("RocksDB iterator error: {}", e))?;
            if key.len() == 32 {
                match bincode::deserialize::<Account>(&value) {
                    Ok(acct) => {
                        let mut addr = [0u8; 32];
                        addr.copy_from_slice(&key);
                        accounts.insert(addr, acct);
                    }
                    Err(e) => {
                        eprintln!("  Warning: skipping corrupt account record: {}", e);
                    }
                }
            }
        }

        let count = objects.len() + ghosts.len() + accounts.len();
        if count > 0 {
            println!(
                "  RocksDB: loaded {} objects, {} ghosts, {} accounts from disk",
                objects.len(),
                ghosts.len(),
                accounts.len()
            );
        }

        // Load nullifiers
        let mut spent_nullifiers = std::collections::HashSet::new();
        let cf_null = db
            .cf_handle(CF_NULLIFIERS)
            .ok_or_else(|| format!("missing column family: {CF_NULLIFIERS}"))?;
        let iter = db.iterator_cf(cf_null, rocksdb::IteratorMode::Start);
        for item in iter {
            let (key, _value) = item.map_err(|e| format!("RocksDB iterator error: {}", e))?;
            if key.len() == 32 {
                let mut nul = [0u8; 32];
                nul.copy_from_slice(&key);
                spent_nullifiers.insert(nul);
            }
        }

        // Load privacy metadata from trie CF
        let cf_trie_meta = db
            .cf_handle(CF_TRIE)
            .ok_or_else(|| format!("missing column family: {CF_TRIE}"))?;
        let note_tree_root = match db.get_cf(cf_trie_meta, PRIVACY_NOTE_ROOT_KEY) {
            Ok(Some(bytes)) if bytes.len() == 32 => {
                let mut root = [0u8; 32];
                root.copy_from_slice(&bytes);
                root
            }
            _ => [0u8; 32],
        };
        let shielded_pool_balance = match db.get_cf(cf_trie_meta, PRIVACY_POOL_BALANCE_KEY) {
            Ok(Some(bytes)) if bytes.len() == 8 => {
                u64::from_le_bytes(bytes[..8].try_into().unwrap())
            }
            _ => 0,
        };
        let note_count = match db.get_cf(cf_trie_meta, PRIVACY_NOTE_COUNT_KEY) {
            Ok(Some(bytes)) if bytes.len() == 8 => {
                u64::from_le_bytes(bytes[..8].try_into().unwrap())
            }
            _ => 0,
        };
        let last_rent_epoch = match db.get_cf(cf_trie_meta, LAST_RENT_EPOCH_KEY) {
            Ok(Some(bytes)) if bytes.len() == 8 => {
                u64::from_le_bytes(bytes[..8].try_into().unwrap())
            }
            _ => 0,
        };
        if !spent_nullifiers.is_empty() {
            println!(
                "  RocksDB: loaded {} nullifiers, pool_balance={}, note_count={}",
                spent_nullifiers.len(),
                shielded_pool_balance,
                note_count
            );
        }

        // Load persisted note commitments (leaf-index ordered).
        let mut note_commitments: BTreeMap<u64, [u8; 32]> = BTreeMap::new();
        let cf_commits = db
            .cf_handle(CF_NOTE_COMMITMENTS)
            .ok_or_else(|| format!("missing column family: {CF_NOTE_COMMITMENTS}"))?;
        let iter = db.iterator_cf(cf_commits, rocksdb::IteratorMode::Start);
        for item in iter {
            let (key, value) = item.map_err(|e| format!("RocksDB iterator error: {}", e))?;
            if key.len() == 8 && value.len() == 32 {
                let idx = u64::from_be_bytes(key[..8].try_into().unwrap());
                let mut commit = [0u8; 32];
                commit.copy_from_slice(&value);
                note_commitments.insert(idx, commit);
            }
        }
        if !note_commitments.is_empty() {
            println!(
                "  RocksDB: loaded {} note commitments",
                note_commitments.len()
            );
        }

        // Load persisted trie or rebuild from scratch
        let trie = {
            let cf_trie = db
                .cf_handle(CF_TRIE)
                .ok_or_else(|| format!("missing column family: {CF_TRIE}"))?;
            match db.get_cf(cf_trie, TRIE_SNAPSHOT_KEY) {
                Ok(Some(bytes)) => match EnergyVerkleTrie::from_bytes(&bytes) {
                    Ok(t) => {
                        println!(
                            "  RocksDB: loaded energy-verkle trie from disk ({} bytes)",
                            bytes.len()
                        );
                        t
                    }
                    Err(e) => {
                        eprintln!("  Warning: trie snapshot corrupt ({}), rebuilding", e);
                        build_energy_trie(&objects, &accounts)
                    }
                },
                _ => {
                    if !objects.is_empty() || !accounts.is_empty() {
                        println!("  RocksDB: no trie snapshot found, rebuilding from state");
                        build_energy_trie(&objects, &accounts)
                    } else {
                        EnergyVerkleTrie::new()
                    }
                }
            }
        };

        // Load Sentinel parameters and votes.
        let mut sentinel_params: BTreeMap<u32, evaporchain_sentinel::BoundedParameter> =
            BTreeMap::new();
        let cf_sp = db
            .cf_handle(CF_SENTINEL_PARAMS)
            .ok_or_else(|| format!("missing column family: {CF_SENTINEL_PARAMS}"))?;
        let iter = db.iterator_cf(cf_sp, rocksdb::IteratorMode::Start);
        for item in iter {
            let (key, value) = item.map_err(|e| format!("RocksDB iterator error: {}", e))?;
            if key.len() == 4 {
                if let Ok(p) =
                    bincode::deserialize::<evaporchain_sentinel::BoundedParameter>(&value)
                {
                    sentinel_params.insert(p.id, p);
                }
            }
        }
        let mut sentinel_votes: BTreeMap<u32, Vec<evaporchain_sentinel::Vote>> = BTreeMap::new();
        let cf_sv = db
            .cf_handle(CF_SENTINEL_VOTES)
            .ok_or_else(|| format!("missing column family: {CF_SENTINEL_VOTES}"))?;
        let iter = db.iterator_cf(cf_sv, rocksdb::IteratorMode::Start);
        for item in iter {
            let (key, value) = item.map_err(|e| format!("RocksDB iterator error: {}", e))?;
            if key.len() == 4 {
                let id = u32::from_be_bytes(key[..4].try_into().unwrap());
                if let Ok(votes) = bincode::deserialize::<Vec<evaporchain_sentinel::Vote>>(&value) {
                    sentinel_votes.insert(id, votes);
                }
            }
        }
        if !sentinel_params.is_empty() {
            println!(
                "  RocksDB: loaded {} Sentinel parameters, {} vote slates",
                sentinel_params.len(),
                sentinel_votes.len()
            );
        }

        // Load validator stake records.
        let mut stakes: HashMap<u64, StakeRecord> = HashMap::new();
        let cf_stk = db
            .cf_handle(CF_STAKES)
            .ok_or_else(|| format!("missing column family: {CF_STAKES}"))?;
        let iter = db.iterator_cf(cf_stk, rocksdb::IteratorMode::Start);
        for item in iter {
            let (key, value) = item.map_err(|e| format!("RocksDB iterator error: {}", e))?;
            if key.len() == 8 {
                if let Ok(record) = bincode::deserialize::<StakeRecord>(&value) {
                    let id = u64::from_be_bytes(key[..8].try_into().unwrap());
                    stakes.insert(id, record);
                }
            }
        }

        // Load delegation records (key = delegator[32] ++ validator_id_be[8]).
        let mut delegations: HashMap<[u8; 40], DelegationRecord> = HashMap::new();
        let cf_del = db
            .cf_handle(CF_DELEGATIONS)
            .ok_or_else(|| format!("missing column family: {CF_DELEGATIONS}"))?;
        let iter = db.iterator_cf(cf_del, rocksdb::IteratorMode::Start);
        for item in iter {
            let (key, value) = item.map_err(|e| format!("RocksDB iterator error: {}", e))?;
            if key.len() == 40 {
                if let Ok(record) = bincode::deserialize::<DelegationRecord>(&value) {
                    let mut k = [0u8; 40];
                    k.copy_from_slice(&key);
                    delegations.insert(k, record);
                }
            }
        }
        if !stakes.is_empty() || !delegations.is_empty() {
            println!(
                "  RocksDB: loaded {} stakes, {} delegations",
                stakes.len(),
                delegations.len()
            );
        }

        Ok(Self {
            db,
            objects,
            ghosts,
            accounts,
            trie,
            dirty_objects: HashSet::new(),
            dirty_accounts: HashSet::new(),
            note_tree_root,
            spent_nullifiers,
            shielded_pool_balance,
            note_count,
            note_commitments,
            last_rent_epoch,
            sentinel_params,
            sentinel_votes,
            stakes,
            delegations,
            pending_batch: std::sync::Mutex::new(None),
            batch_undo: None,
        })
    }

    /// Start buffering writes for atomic commit. Call `commit_batch()` to flush.
    pub fn begin_batch(&mut self) {
        *self.pending_batch.lock().unwrap() = Some(WriteBatch::default());
        self.sync_dirty_to_trie();
        self.batch_undo = Some(BatchUndoLog {
            objects: Vec::new(),
            accounts: Vec::new(),
            ghosts: Vec::new(),
            dirty_objects: self.dirty_objects.clone(),
            dirty_accounts: self.dirty_accounts.clone(),
            trie_snapshot: self.trie.to_bytes(),
            note_tree_root: self.note_tree_root,
            nullifiers_snapshot: self.spent_nullifiers.clone(),
            shielded_pool_balance: self.shielded_pool_balance,
            note_count: self.note_count,
            note_commitments_snapshot: self.note_commitments.clone(),
        });
    }

    /// Atomically write all buffered mutations to disk.
    pub fn commit_batch(&mut self) -> Result<(), String> {
        self.batch_undo = None;
        let batch = self
            .pending_batch
            .lock()
            .unwrap()
            .take()
            .ok_or("no active batch")?;
        self.db
            .write(batch)
            .map_err(|e| format!("WriteBatch commit failed: {e}"))
    }

    /// Discard any buffered writes and revert in-memory state.
    pub fn rollback_batch(&mut self) {
        *self.pending_batch.lock().unwrap() = None;
        if let Some(undo) = self.batch_undo.take() {
            for (id, old_val) in undo.objects.into_iter().rev() {
                match old_val {
                    Some(obj) => {
                        self.objects.insert(id, obj);
                    }
                    None => {
                        self.objects.remove(&id);
                    }
                }
            }
            for (addr, old_val) in undo.accounts.into_iter().rev() {
                match old_val {
                    Some(acc) => {
                        self.accounts.insert(addr, acc);
                    }
                    None => {
                        self.accounts.remove(&addr);
                    }
                }
            }
            for (id, old_val) in undo.ghosts.into_iter().rev() {
                match old_val {
                    Some(ghost) => {
                        self.ghosts.insert(id, ghost);
                    }
                    None => {
                        self.ghosts.remove(&id);
                    }
                }
            }
            self.dirty_objects = undo.dirty_objects;
            self.dirty_accounts = undo.dirty_accounts;
            if let Ok(trie) = EnergyVerkleTrie::from_bytes(&undo.trie_snapshot) {
                self.trie = trie;
            }
            // Revert privacy state
            self.note_tree_root = undo.note_tree_root;
            self.spent_nullifiers = undo.nullifiers_snapshot;
            self.shielded_pool_balance = undo.shielded_pool_balance;
            self.note_count = undo.note_count;
            self.note_commitments = undo.note_commitments_snapshot;
        }
    }

    /// Returns true if the database has any accounts (i.e., not a fresh start).
    pub fn has_data(&self) -> bool {
        !self.accounts.is_empty()
    }

    /// Get a column family handle, panicking with a clear message if missing.
    /// Column families are created at open time, so this should never fail
    /// unless the DB is corrupted.
    fn cf(&self, name: &str) -> &ColumnFamily {
        self.db.cf_handle(name).unwrap_or_else(|| {
            panic!(
                "FATAL: RocksDB column family '{}' missing — database may be corrupted. \
                 Delete the data directory and restart.",
                name
            )
        })
    }

    fn persist_object(&self, obj: &StateObject) {
        let value = match bincode::serialize(obj) {
            Ok(v) => v,
            Err(e) => fatal_persistence_error("serialize_object", e),
        };
        let mut guard = self.pending_batch.lock().unwrap();
        if let Some(ref mut batch) = *guard {
            let cf = self.db.cf_handle(CF_OBJECTS).unwrap();
            batch.put_cf(cf, obj.id, &value);
        } else {
            drop(guard);
            let cf = self.cf(CF_OBJECTS);
            if let Err(e) = self.db.put_cf(cf, obj.id, value) {
                fatal_persistence_error("write_object_to_rocksdb", e);
            }
        }
    }

    fn delete_object_disk(&self, id: &ObjectId) {
        let mut guard = self.pending_batch.lock().unwrap();
        if let Some(ref mut batch) = *guard {
            let cf = self.db.cf_handle(CF_OBJECTS).unwrap();
            batch.delete_cf(cf, id);
        } else {
            drop(guard);
            let cf = self.cf(CF_OBJECTS);
            if let Err(e) = self.db.delete_cf(cf, id) {
                fatal_persistence_error("delete_object_from_rocksdb", e);
            }
        }
    }

    fn persist_ghost(&self, ghost: &GhostRecord) {
        let value = match bincode::serialize(ghost) {
            Ok(v) => v,
            Err(e) => fatal_persistence_error("serialize_ghost", e),
        };
        let mut guard = self.pending_batch.lock().unwrap();
        if let Some(ref mut batch) = *guard {
            let cf = self.db.cf_handle(CF_GHOSTS).unwrap();
            batch.put_cf(cf, ghost.object_id, &value);
        } else {
            drop(guard);
            let cf = self.cf(CF_GHOSTS);
            if let Err(e) = self.db.put_cf(cf, ghost.object_id, value) {
                fatal_persistence_error("write_ghost_to_rocksdb", e);
            }
        }
    }

    fn delete_ghost_disk(&self, id: &ObjectId) {
        let mut guard = self.pending_batch.lock().unwrap();
        if let Some(ref mut batch) = *guard {
            let cf = self.db.cf_handle(CF_GHOSTS).unwrap();
            batch.delete_cf(cf, id);
        } else {
            drop(guard);
            let cf = self.cf(CF_GHOSTS);
            if let Err(e) = self.db.delete_cf(cf, id) {
                fatal_persistence_error("delete_ghost_from_rocksdb", e);
            }
        }
    }

    fn persist_account(&self, account: &Account) {
        let value = match bincode::serialize(account) {
            Ok(v) => v,
            Err(e) => fatal_persistence_error("serialize_account", e),
        };
        let mut guard = self.pending_batch.lock().unwrap();
        if let Some(ref mut batch) = *guard {
            let cf = self.db.cf_handle(CF_ACCOUNTS).unwrap();
            batch.put_cf(cf, account.address, &value);
        } else {
            drop(guard);
            let cf = self.cf(CF_ACCOUNTS);
            if let Err(e) = self.db.put_cf(cf, account.address, value) {
                fatal_persistence_error("write_account_to_rocksdb", e);
            }
        }
    }

    fn sync_dirty_to_trie(&mut self) {
        let mut dirty_objs: Vec<_> = self.dirty_objects.drain().collect();
        dirty_objs.sort();
        for id in dirty_objs {
            let key = trie_key_for_object(&id);
            if let Some(obj) = self.objects.get(&id) {
                self.trie.insert(
                    key,
                    trie_value_for_object(obj),
                    obj.energy,
                    obj.half_life,
                    obj.last_refreshed,
                );
            } else {
                self.trie.delete(&key);
            }
        }
        let mut dirty_accts: Vec<_> = self.dirty_accounts.drain().collect();
        dirty_accts.sort();
        for addr in dirty_accts {
            let key = trie_key_for_account(&addr);
            if let Some(acc) = self.accounts.get(&addr) {
                self.trie
                    .insert(key, trie_value_for_account(acc), u64::MAX, u64::MAX, 0);
            }
        }
    }

    fn persist_trie(&self) {
        let cf = self.cf(CF_TRIE);
        let bytes = self.trie.to_bytes();
        if let Err(e) = self.db.put_cf(cf, TRIE_SNAPSHOT_KEY, bytes) {
            fatal_persistence_error("write_trie_snapshot_to_rocksdb", e);
        }
    }

    fn persist_nullifier(&self, nullifier: &[u8; 32]) {
        let mut guard = self.pending_batch.lock().unwrap();
        if let Some(ref mut batch) = *guard {
            let cf = self.db.cf_handle(CF_NULLIFIERS).unwrap();
            batch.put_cf(cf, nullifier, [1u8]);
        } else {
            drop(guard);
            let cf = self.cf(CF_NULLIFIERS);
            if let Err(e) = self.db.put_cf(cf, nullifier, [1u8]) {
                fatal_persistence_error("write_nullifier_to_rocksdb", e);
            }
        }
    }

    fn persist_note_commitment(&self, index: u64, commitment: &[u8; 32]) {
        let key = index.to_be_bytes();
        let mut guard = self.pending_batch.lock().unwrap();
        if let Some(ref mut batch) = *guard {
            let cf = self.db.cf_handle(CF_NOTE_COMMITMENTS).unwrap();
            batch.put_cf(cf, key, commitment);
        } else {
            drop(guard);
            let cf = self.cf(CF_NOTE_COMMITMENTS);
            if let Err(e) = self.db.put_cf(cf, key, commitment) {
                fatal_persistence_error("write_note_commitment_to_rocksdb", e);
            }
        }
    }

    fn persist_privacy_metadata(&self) {
        let cf = self.cf(CF_TRIE);
        if let Err(e) = self
            .db
            .put_cf(cf, PRIVACY_NOTE_ROOT_KEY, self.note_tree_root)
        {
            fatal_persistence_error("write_privacy_note_tree_root", e);
        }
        if let Err(e) = self.db.put_cf(
            cf,
            PRIVACY_POOL_BALANCE_KEY,
            self.shielded_pool_balance.to_le_bytes(),
        ) {
            fatal_persistence_error("write_privacy_pool_balance", e);
        }
        if let Err(e) = self
            .db
            .put_cf(cf, PRIVACY_NOTE_COUNT_KEY, self.note_count.to_le_bytes())
        {
            fatal_persistence_error("write_privacy_note_count", e);
        }
    }

    fn persist_stake(&self, record: &StakeRecord) {
        let key = record.validator_id.to_be_bytes();
        let value = match bincode::serialize(record) {
            Ok(v) => v,
            Err(e) => fatal_persistence_error("serialize_stake", e),
        };
        let mut guard = self.pending_batch.lock().unwrap();
        if let Some(ref mut batch) = *guard {
            let cf = self.db.cf_handle(CF_STAKES).unwrap();
            batch.put_cf(cf, key, &value);
        } else {
            drop(guard);
            let cf = self.cf(CF_STAKES);
            if let Err(e) = self.db.put_cf(cf, key, value) {
                fatal_persistence_error("write_stake_to_rocksdb", e);
            }
        }
    }

    fn delete_stake_disk(&self, validator_id: u64) {
        let key = validator_id.to_be_bytes();
        let mut guard = self.pending_batch.lock().unwrap();
        if let Some(ref mut batch) = *guard {
            let cf = self.db.cf_handle(CF_STAKES).unwrap();
            batch.delete_cf(cf, key);
        } else {
            drop(guard);
            let cf = self.cf(CF_STAKES);
            if let Err(e) = self.db.delete_cf(cf, key) {
                fatal_persistence_error("delete_stake_from_rocksdb", e);
            }
        }
    }

    fn persist_delegation(&self, record: &DelegationRecord) {
        let key = delegation_key(&record.delegator, record.validator_id);
        let value = match bincode::serialize(record) {
            Ok(v) => v,
            Err(e) => fatal_persistence_error("serialize_delegation", e),
        };
        let mut guard = self.pending_batch.lock().unwrap();
        if let Some(ref mut batch) = *guard {
            let cf = self.db.cf_handle(CF_DELEGATIONS).unwrap();
            batch.put_cf(cf, key, &value);
        } else {
            drop(guard);
            let cf = self.cf(CF_DELEGATIONS);
            if let Err(e) = self.db.put_cf(cf, key, value) {
                fatal_persistence_error("write_delegation_to_rocksdb", e);
            }
        }
    }

    fn delete_delegation_disk(&self, delegator: &AccountAddress, validator_id: u64) {
        let key = delegation_key(delegator, validator_id);
        let mut guard = self.pending_batch.lock().unwrap();
        if let Some(ref mut batch) = *guard {
            let cf = self.db.cf_handle(CF_DELEGATIONS).unwrap();
            batch.delete_cf(cf, key);
        } else {
            drop(guard);
            let cf = self.cf(CF_DELEGATIONS);
            if let Err(e) = self.db.delete_cf(cf, key) {
                fatal_persistence_error("delete_delegation_from_rocksdb", e);
            }
        }
    }
}

/// Composite key for delegation records: delegator_address[32] ++ validator_id_be[8].
fn delegation_key(delegator: &AccountAddress, validator_id: u64) -> [u8; 40] {
    let mut key = [0u8; 40];
    key[..32].copy_from_slice(delegator);
    key[32..].copy_from_slice(&validator_id.to_be_bytes());
    key
}

/// Attempt to deserialize a ghost record from legacy format (no mmr_position field).
fn deserialize_legacy_ghost(
    data: &[u8],
    id: &ObjectId,
) -> Result<GhostRecord, Box<bincode::ErrorKind>> {
    // Legacy bincode layout (no mmr_position):
    // object_id: [u8; 32], owner: [u8; 32], evaporated_at: u64, data_hash: [u8; 32], original_data: Vec<u8>
    // Minimum size: 32 + 32 + 8 + 32 + 8 (vec length prefix) = 112
    if data.len() < 112 {
        return Err(Box::new(bincode::ErrorKind::Custom(
            "legacy ghost record too short".into(),
        )));
    }

    let mut offset = 0;
    let mut object_id = [0u8; 32];
    object_id.copy_from_slice(&data[offset..offset + 32]);
    offset += 32;

    let mut owner = [0u8; 32];
    owner.copy_from_slice(&data[offset..offset + 32]);
    offset += 32;

    let evaporated_at = u64::from_le_bytes(data[offset..offset + 8].try_into().map_err(|_| {
        Box::new(bincode::ErrorKind::Custom(
            "invalid evaporated_at bytes".into(),
        ))
    })?);
    offset += 8;

    let mut data_hash = [0u8; 32];
    data_hash.copy_from_slice(&data[offset..offset + 32]);
    offset += 32;

    // bincode encodes Vec<u8> with a u64 length prefix
    let data_len = u64::from_le_bytes(
        data[offset..offset + 8]
            .try_into()
            .map_err(|_| Box::new(bincode::ErrorKind::Custom("invalid data_len bytes".into())))?,
    ) as usize;
    offset += 8;

    if offset + data_len > data.len() {
        return Err(Box::new(bincode::ErrorKind::Custom(
            "legacy ghost data length mismatch".into(),
        )));
    }

    let original_data = data[offset..offset + data_len].to_vec();

    Ok(GhostRecord {
        object_id: *id,
        owner,
        evaporated_at,
        data_hash,
        original_data: Some(original_data),
        mmr_position: None,
        original_half_life: None,
    })
}

impl StateDB for RocksDBStateDB {
    fn get_object(&self, id: &ObjectId) -> Option<&StateObject> {
        self.objects.get(id)
    }

    fn get_object_mut(&mut self, id: &ObjectId) -> Option<&mut StateObject> {
        if self.objects.contains_key(id) {
            if let Some(ref mut undo) = self.batch_undo {
                undo.objects.push((*id, Some(self.objects[id].clone())));
            }
            self.dirty_objects.insert(*id);
        }
        self.objects.get_mut(id)
    }

    fn put_object(&mut self, obj: StateObject) {
        if let Some(ref mut undo) = self.batch_undo {
            undo.objects
                .push((obj.id, self.objects.get(&obj.id).cloned()));
        }
        self.persist_object(&obj);
        let key = trie_key_for_object(&obj.id);
        let value = trie_value_for_object(&obj);
        self.trie
            .insert(key, value, obj.energy, obj.half_life, obj.last_refreshed);
        self.dirty_objects.remove(&obj.id);
        self.objects.insert(obj.id, obj);
    }

    fn delete_object(&mut self, id: &ObjectId) -> Option<StateObject> {
        if let Some(ref mut undo) = self.batch_undo {
            undo.objects.push((*id, self.objects.get(id).cloned()));
        }
        self.delete_object_disk(id);
        let key = trie_key_for_object(id);
        self.trie.delete(&key);
        self.dirty_objects.remove(id);
        self.objects.remove(id)
    }

    fn put_ghost(&mut self, record: GhostRecord) {
        if let Some(ref mut undo) = self.batch_undo {
            undo.ghosts.push((
                record.object_id,
                self.ghosts.get(&record.object_id).cloned(),
            ));
        }
        self.persist_ghost(&record);
        self.ghosts.insert(record.object_id, record);
    }

    fn get_ghost(&self, id: &ObjectId) -> Option<&GhostRecord> {
        self.ghosts.get(id)
    }

    fn remove_ghost(&mut self, id: &ObjectId) -> Option<GhostRecord> {
        if let Some(ref mut undo) = self.batch_undo {
            undo.ghosts.push((*id, self.ghosts.get(id).cloned()));
        }
        self.delete_ghost_disk(id);
        self.ghosts.remove(id)
    }

    fn all_object_ids(&self) -> Vec<ObjectId> {
        self.objects.keys().copied().collect()
    }

    fn object_count(&self) -> usize {
        self.objects.len()
    }

    fn ghost_count(&self) -> usize {
        self.ghosts.len()
    }

    fn all_ghost_ids(&self) -> Vec<ObjectId> {
        self.ghosts.keys().copied().collect()
    }

    fn get_account(&self, addr: &AccountAddress) -> Option<&Account> {
        self.accounts.get(addr)
    }

    fn get_account_mut(&mut self, addr: &AccountAddress) -> Option<&mut Account> {
        if self.accounts.contains_key(addr) {
            if let Some(ref mut undo) = self.batch_undo {
                undo.accounts
                    .push((*addr, Some(self.accounts[addr].clone())));
            }
            self.dirty_accounts.insert(*addr);
        }
        self.accounts.get_mut(addr)
    }

    fn put_account(&mut self, account: Account) {
        if let Some(ref mut undo) = self.batch_undo {
            undo.accounts.push((
                account.address,
                self.accounts.get(&account.address).cloned(),
            ));
        }
        self.persist_account(&account);
        let key = trie_key_for_account(&account.address);
        let value = trie_value_for_account(&account);
        self.trie.insert(key, value, u64::MAX, u64::MAX, 0);
        self.dirty_accounts.remove(&account.address);
        self.accounts.insert(account.address, account);
    }

    fn delete_account(&mut self, addr: &AccountAddress) -> Option<Account> {
        if let Some(ref mut undo) = self.batch_undo {
            undo.accounts
                .push((*addr, self.accounts.get(addr).cloned()));
        }
        let key = trie_key_for_account(addr);
        self.trie.delete(&key);
        self.dirty_accounts.remove(addr);
        let cf = self.db.cf_handle("accounts").unwrap();
        let _ = self.db.delete_cf(cf, addr);
        self.accounts.remove(addr)
    }

    fn get_or_create_account(&mut self, addr: &AccountAddress) -> &mut Account {
        if !self.accounts.contains_key(addr) {
            if let Some(ref mut undo) = self.batch_undo {
                undo.accounts.push((*addr, None));
            }
            let account = Account {
                address: *addr,
                balance: 0,
                nonce: 0,
            storage_deposit: 0,
            storage_bytes: 0,
            last_touched_epoch: 0,
            };
            self.persist_account(&account);
            let key = trie_key_for_account(&account.address);
            let value = trie_value_for_account(&account);
            self.trie.insert(key, value, u64::MAX, u64::MAX, 0);
            self.accounts.insert(*addr, account);
        } else if let Some(ref mut undo) = self.batch_undo {
            undo.accounts
                .push((*addr, Some(self.accounts[addr].clone())));
        }
        self.dirty_accounts.insert(*addr);
        self.accounts
            .get_mut(addr)
            .expect("account must exist: just inserted above")
    }

    fn all_account_addresses(&self) -> Vec<AccountAddress> {
        self.accounts.keys().copied().collect()
    }

    fn compute_state_root(&mut self) -> [u8; 32] {
        self.sync_dirty_to_trie();
        self.trie.root()
    }

    fn compress_cold_subtrees(&mut self) -> u32 {
        self.sync_dirty_to_trie();
        self.trie.compress_cold()
    }

    fn trie_health(&mut self) -> TrieHealth {
        self.sync_dirty_to_trie();
        self.trie.health()
    }

    fn put_note_tree_root(&mut self, root: [u8; 32]) {
        self.note_tree_root = root;
        self.persist_privacy_metadata();
    }

    fn get_note_tree_root(&self) -> [u8; 32] {
        self.note_tree_root
    }

    fn spend_nullifier(&mut self, nullifier: &[u8; 32]) -> bool {
        let is_new = self.spent_nullifiers.insert(*nullifier);
        if is_new {
            self.persist_nullifier(nullifier);
        }
        is_new
    }

    fn is_nullifier_spent(&self, nullifier: &[u8; 32]) -> bool {
        self.spent_nullifiers.contains(nullifier)
    }

    fn nullifier_count(&self) -> usize {
        self.spent_nullifiers.len()
    }

    fn all_nullifiers(&self) -> Vec<[u8; 32]> {
        self.spent_nullifiers.iter().copied().collect()
    }

    fn put_shielded_pool_balance(&mut self, balance: u64) {
        self.shielded_pool_balance = balance;
        self.persist_privacy_metadata();
    }

    fn get_shielded_pool_balance(&self) -> u64 {
        self.shielded_pool_balance
    }

    fn put_note_count(&mut self, count: u64) {
        self.note_count = count;
        self.persist_privacy_metadata();
    }

    fn get_note_count(&self) -> u64 {
        self.note_count
    }

    fn append_note_commitment(&mut self, index: u64, commitment: [u8; 32]) {
        // Idempotent: writing the same (index, commitment) twice is harmless.
        // The cache and disk both end up with the same value.
        self.note_commitments.insert(index, commitment);
        self.persist_note_commitment(index, &commitment);
    }

    fn get_all_note_commitments(&self) -> Vec<[u8; 32]> {
        self.note_commitments.values().copied().collect()
    }

    fn get_last_rent_epoch(&self) -> u64 {
        self.last_rent_epoch
    }

    fn put_last_rent_epoch(&mut self, epoch: u64) {
        self.last_rent_epoch = epoch;
        let cf = self.cf(CF_TRIE);
        if let Err(e) = self.db.put_cf(cf, LAST_RENT_EPOCH_KEY, epoch.to_le_bytes()) {
            fatal_persistence_error("write_last_rent_epoch", e);
        }
    }

    fn get_stake(&self, validator_id: u64) -> Option<&StakeRecord> {
        self.stakes.get(&validator_id)
    }

    fn put_stake(&mut self, record: StakeRecord) {
        self.persist_stake(&record);
        self.stakes.insert(record.validator_id, record);
    }

    fn remove_stake(&mut self, validator_id: u64) -> Option<StakeRecord> {
        self.delete_stake_disk(validator_id);
        self.stakes.remove(&validator_id)
    }

    fn all_stakes(&self) -> Vec<&StakeRecord> {
        self.stakes.values().collect()
    }

    fn get_delegation(
        &self,
        delegator: &AccountAddress,
        validator_id: u64,
    ) -> Option<&DelegationRecord> {
        self.delegations
            .get(&delegation_key(delegator, validator_id))
    }

    fn put_delegation(&mut self, record: DelegationRecord) {
        self.persist_delegation(&record);
        let key = delegation_key(&record.delegator, record.validator_id);
        self.delegations.insert(key, record);
    }

    fn remove_delegation(
        &mut self,
        delegator: &AccountAddress,
        validator_id: u64,
    ) -> Option<DelegationRecord> {
        self.delete_delegation_disk(delegator, validator_id);
        self.delegations
            .remove(&delegation_key(delegator, validator_id))
    }

    fn delegations_for_validator(&self, validator_id: u64) -> Vec<&DelegationRecord> {
        let id_be = validator_id.to_be_bytes();
        self.delegations
            .iter()
            .filter(|(k, _)| k[32..] == id_be)
            .map(|(_, v)| v)
            .collect()
    }

    fn delegations_for_delegator(&self, delegator: &AccountAddress) -> Vec<&DelegationRecord> {
        self.delegations
            .iter()
            .filter(|(k, _)| &k[..32] == delegator.as_slice())
            .map(|(_, v)| v)
            .collect()
    }

    fn all_delegations(&self) -> Vec<&DelegationRecord> {
        self.delegations.values().collect()
    }

    fn prove_account(&mut self, addr: &AccountAddress) -> evaporchain_crypto::EnergyVerkleProof {
        self.sync_dirty_to_trie();
        let key = trie_key_for_account(addr);
        self.trie.prove(&key)
    }

    fn prove_object(&mut self, id: &ObjectId) -> evaporchain_crypto::EnergyVerkleProof {
        self.sync_dirty_to_trie();
        let key = trie_key_for_object(id);
        self.trie.prove(&key)
    }

    fn trie_snapshot(&mut self) -> Vec<u8> {
        self.sync_dirty_to_trie();
        self.trie.to_bytes()
    }

    fn load_trie_snapshot(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.trie = EnergyVerkleTrie::from_bytes(bytes)?;
        self.dirty_objects.clear();
        self.dirty_accounts.clear();
        Ok(())
    }

    fn prune_before_height(&mut self, height: u64) -> u64 {
        // Collect ghost IDs whose evaporated_at epoch is strictly before `height`.
        let ids_to_prune: Vec<ObjectId> = self
            .ghosts
            .iter()
            .filter(|(_, ghost)| ghost.evaporated_at < height)
            .map(|(id, _)| *id)
            .collect();

        let count = ids_to_prune.len() as u64;
        if count == 0 {
            return 0;
        }

        // Delete from RocksDB in a single batch for atomicity.
        let cf = self.db.cf_handle(CF_GHOSTS).expect("ghosts CF must exist");
        let mut batch = WriteBatch::default();
        for id in &ids_to_prune {
            batch.delete_cf(cf, id);
        }
        if let Err(e) = self.db.write(batch) {
            fatal_persistence_error("prune_before_height_writebatch_commit", e);
        }

        // Remove from in-memory cache.
        for id in &ids_to_prune {
            self.ghosts.remove(id);
        }

        count
    }

    fn get_proposal(&self, _proposal_id: u64) -> Option<&evaporchain_types::GovernanceProposal> {
        None
    }
    fn put_proposal(&mut self, _proposal: evaporchain_types::GovernanceProposal) {}
    fn all_proposals(&self) -> Vec<&evaporchain_types::GovernanceProposal> {
        Vec::new()
    }
    fn get_governance_param(&self, _key: &str) -> Option<&str> {
        None
    }
    fn put_governance_param(&mut self, _key: String, _value: String) {}

    fn commit_state_snapshot(&mut self, _height: u64) {}
    fn get_account_at_height(
        &self,
        _address: &evaporchain_types::AccountAddress,
        _height: u64,
    ) -> Option<evaporchain_types::Account> {
        None
    }
    fn get_object_at_height(
        &self,
        _id: &evaporchain_types::ObjectId,
        _height: u64,
    ) -> Option<evaporchain_types::StateObject> {
        None
    }
    fn earliest_snapshot_height(&self) -> Option<u64> {
        None
    }
    fn latest_snapshot_height(&self) -> Option<u64> {
        None
    }
    fn prune_snapshots_before(&mut self, _height: u64) {}

    // ─── Sentinel autonomic governance (write-through to RocksDB) ───────

    fn get_sentinel_param(&self, id: u32) -> Option<evaporchain_sentinel::BoundedParameter> {
        self.sentinel_params.get(&id).copied()
    }

    fn put_sentinel_param(&mut self, param: evaporchain_sentinel::BoundedParameter) {
        let key = param.id.to_be_bytes();
        let val = match bincode::serialize(&param) {
            Ok(v) => v,
            Err(e) => fatal_persistence_error("serialize_sentinel_param", e),
        };
        let cf = self.cf(CF_SENTINEL_PARAMS);
        if let Err(e) = self.db.put_cf(cf, key, val) {
            fatal_persistence_error("put_sentinel_param", e);
        }
        self.sentinel_params.insert(param.id, param);
        self.sentinel_votes.entry(param.id).or_default();
    }

    fn all_sentinel_params(&self) -> Vec<evaporchain_sentinel::BoundedParameter> {
        self.sentinel_params.values().copied().collect()
    }

    fn get_sentinel_votes(&self, id: u32) -> Vec<evaporchain_sentinel::Vote> {
        self.sentinel_votes.get(&id).cloned().unwrap_or_default()
    }

    fn put_sentinel_votes(&mut self, parameter_id: u32, votes: Vec<evaporchain_sentinel::Vote>) {
        let key = parameter_id.to_be_bytes();
        let val = match bincode::serialize(&votes) {
            Ok(v) => v,
            Err(e) => fatal_persistence_error("serialize_sentinel_votes", e),
        };
        let cf = self.cf(CF_SENTINEL_VOTES);
        if let Err(e) = self.db.put_cf(cf, key, val) {
            fatal_persistence_error("put_sentinel_votes", e);
        }
        self.sentinel_votes.insert(parameter_id, votes);
    }
}

/// Flush account changes back to RocksDB after mutable borrows.
/// Call this after any block execution that modifies accounts via get_account_mut()
/// or get_or_create_account() followed by balance/nonce changes.
impl RocksDBStateDB {
    pub fn flush_accounts(&mut self) {
        let cf = self.cf(CF_ACCOUNTS);
        for account in self.accounts.values() {
            let value = match bincode::serialize(account) {
                Ok(v) => v,
                Err(e) => fatal_persistence_error("flush_serialize_account", e),
            };
            if let Err(e) = self.db.put_cf(cf, account.address, value) {
                fatal_persistence_error("flush_account_to_rocksdb", e);
            }
        }
        self.sync_dirty_to_trie();
        self.persist_trie();
    }

    pub fn flush_objects(&mut self) {
        let cf = self.cf(CF_OBJECTS);
        for obj in self.objects.values() {
            let value = match bincode::serialize(obj) {
                Ok(v) => v,
                Err(e) => fatal_persistence_error("flush_serialize_object", e),
            };
            if let Err(e) = self.db.put_cf(cf, obj.id, value) {
                fatal_persistence_error("flush_object_to_rocksdb", e);
            }
        }
        self.sync_dirty_to_trie();
        self.persist_trie();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_types::{Account, DelegationRecord, ObjectState, StakeRecord, StateObject};

    fn tmp_db() -> RocksDBStateDB {
        let dir = tempfile::tempdir().unwrap();
        RocksDBStateDB::open(dir.path()).unwrap()
    }

    fn make_obj(b: u8, energy: u64) -> StateObject {
        let mut id = [0u8; 32];
        id[0] = b;
        StateObject {
            id,
            owner: [0u8; 32],
            energy,
            half_life: 100,
            created_at: 0,
            last_refreshed: 0,
            state: ObjectState::Active,
            grace_epoch: None,
            data: vec![b],
            decay_curve: None,
            lad_mode: None,
        }
    }

    fn make_account(b: u8, balance: u64) -> Account {
        let mut addr = [0u8; 32];
        addr[0] = b;
        Account { address: addr, balance, nonce: 0, storage_deposit: 0, storage_bytes: 0, last_touched_epoch: 0 }
    }

    #[test]
    fn test_object_roundtrip() {
        let mut db = tmp_db();
        let obj = make_obj(1, 500);
        db.put_object(obj.clone());
        let loaded = db.get_object(&obj.id).unwrap();
        assert_eq!(loaded.energy, 500);
        assert_eq!(loaded.data, vec![1]);
    }

    #[test]
    fn test_account_roundtrip() {
        let mut db = tmp_db();
        let acc = make_account(1, 1000);
        db.put_account(acc.clone());
        let loaded = db.get_account(&acc.address).unwrap();
        assert_eq!(loaded.balance, 1000);
    }

    #[test]
    fn test_ghost_roundtrip() {
        let mut db = tmp_db();
        let ghost = GhostRecord {
            object_id: [5u8; 32],
            owner: [1u8; 32],
            evaporated_at: 42,
            data_hash: [0xAA; 32],
            original_data: Some(vec![1, 2, 3]),
            mmr_position: None,
            original_half_life: Some(200),
        };
        db.put_ghost(ghost.clone());
        assert_eq!(db.ghost_count(), 1);
        let loaded = db.get_ghost(&ghost.object_id).unwrap();
        assert_eq!(loaded.evaporated_at, 42);
        assert_eq!(loaded.original_half_life, Some(200));
    }

    #[test]
    fn test_write_batch_atomic_commit() {
        let mut db = tmp_db();
        db.begin_batch();
        db.put_object(make_obj(1, 100));
        db.put_object(make_obj(2, 200));
        db.put_account(make_account(1, 500));
        db.commit_batch().unwrap();

        assert_eq!(db.object_count(), 2);
        assert_eq!(db.get_object(&make_obj(1, 0).id).unwrap().energy, 100);
        assert_eq!(
            db.get_account(&make_account(1, 0).address).unwrap().balance,
            500
        );
    }

    #[test]
    fn test_write_batch_rollback() {
        let mut db = tmp_db();
        db.put_object(make_obj(1, 100));
        db.put_account(make_account(1, 500));
        assert_eq!(db.object_count(), 1);

        db.begin_batch();
        db.put_object(make_obj(2, 200));
        db.put_account(make_account(2, 999));
        db.rollback_batch();

        // Rollback must revert in-memory state to pre-batch values
        assert_eq!(db.object_count(), 1);
        assert!(db.get_object(&make_obj(2, 0).id).is_none());
        assert_eq!(
            db.get_account(&make_account(1, 0).address).unwrap().balance,
            500
        );
        assert!(db.get_account(&make_account(2, 0).address).is_none());
    }

    #[test]
    fn test_persistence_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut db = RocksDBStateDB::open(dir.path()).unwrap();
            db.put_object(make_obj(1, 999));
            db.put_account(make_account(1, 777));
        }
        {
            let db = RocksDBStateDB::open(dir.path()).unwrap();
            assert_eq!(db.get_object(&make_obj(1, 0).id).unwrap().energy, 999);
            assert_eq!(
                db.get_account(&make_account(1, 0).address).unwrap().balance,
                777
            );
        }
    }

    #[test]
    fn test_delete_object() {
        let mut db = tmp_db();
        let obj = make_obj(1, 100);
        db.put_object(obj.clone());
        assert_eq!(db.object_count(), 1);
        db.delete_object(&obj.id);
        assert_eq!(db.object_count(), 0);
        assert!(db.get_object(&obj.id).is_none());
    }

    #[test]
    fn test_nullifier_operations() {
        let mut db = tmp_db();
        let nf = [0xAA; 32];
        assert!(!db.is_nullifier_spent(&nf));
        assert!(db.spend_nullifier(&nf));
        assert!(db.is_nullifier_spent(&nf));
        assert!(!db.spend_nullifier(&nf)); // already spent
        assert_eq!(db.nullifier_count(), 1);
        assert_eq!(db.all_nullifiers().len(), 1);
    }

    #[test]
    fn test_state_root_deterministic() {
        let mut db = tmp_db();
        db.put_object(make_obj(1, 100));
        db.put_object(make_obj(2, 200));
        let root1 = db.compute_state_root();
        let root2 = db.compute_state_root();
        assert_eq!(root1, root2);
        assert_ne!(root1, [0u8; 32]);
    }

    #[test]
    fn test_commit_batch_without_begin_fails() {
        let mut db = tmp_db();
        assert!(db.commit_batch().is_err());
    }

    #[test]
    fn test_privacy_state_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();

        let nul1 = [0xAAu8; 32];
        let nul2 = [0xBBu8; 32];
        let root = [0xCCu8; 32];

        {
            let mut db = RocksDBStateDB::open(&path).unwrap();
            db.put_note_tree_root(root);
            db.put_shielded_pool_balance(42_000);
            db.put_note_count(7);
            assert!(db.spend_nullifier(&nul1));
            assert!(db.spend_nullifier(&nul2));
            assert!(!db.spend_nullifier(&nul1)); // duplicate
        }

        // Reopen and verify everything survived
        let db = RocksDBStateDB::open(&path).unwrap();
        assert_eq!(db.get_note_tree_root(), root);
        assert_eq!(db.get_shielded_pool_balance(), 42_000);
        assert_eq!(db.get_note_count(), 7);
        assert!(db.is_nullifier_spent(&nul1));
        assert!(db.is_nullifier_spent(&nul2));
        assert!(!db.is_nullifier_spent(&[0xDD; 32]));
        assert_eq!(db.nullifier_count(), 2);
    }

    #[test]
    fn test_prune_before_height() {
        let mut db = tmp_db();

        // Insert ghosts at epochs 10, 50, 100, 200
        for epoch in [10u64, 50, 100, 200] {
            let mut id = [0u8; 32];
            id[0] = epoch as u8;
            db.put_ghost(GhostRecord {
                object_id: id,
                owner: [0u8; 32],
                evaporated_at: epoch,
                data_hash: [0u8; 32],
                original_data: None,
                mmr_position: None,
                original_half_life: None,
            });
        }
        assert_eq!(db.ghost_count(), 4);

        // Prune ghosts evaporated before height 100 — removes epochs 10 and 50
        let pruned = db.prune_before_height(100);
        assert_eq!(pruned, 2);
        assert_eq!(db.ghost_count(), 2);

        // Ghost at epoch 100 survives (strictly less than)
        let mut id100 = [0u8; 32];
        id100[0] = 100;
        assert!(db.get_ghost(&id100).is_some());

        // Ghost at epoch 200 survives
        let mut id200 = [0u8; 32];
        id200[0] = 200u8;
        assert!(db.get_ghost(&id200).is_some());
    }

    fn make_stake(validator_id: u64, amount: u64) -> StakeRecord {
        StakeRecord {
            validator_id,
            validator_address: [validator_id as u8; 32],
            staked_amount: amount,
            staked_at_epoch: 1,
            unbonding_epoch: None,
            slashed_amount: 0,
        }
    }

    fn make_delegation(delegator_byte: u8, validator_id: u64, amount: u64) -> DelegationRecord {
        DelegationRecord {
            delegator: [delegator_byte; 32],
            validator_id,
            amount,
            delegated_at_epoch: 1,
            unbonding_amount: 0,
            unbonding_epoch: None,
        }
    }

    #[test]
    fn test_stake_roundtrip() {
        let mut db = tmp_db();
        let s = make_stake(1, 250_000);
        db.put_stake(s.clone());
        let loaded = db.get_stake(1).unwrap();
        assert_eq!(loaded.staked_amount, 250_000);
        assert_eq!(db.all_stakes().len(), 1);
    }

    #[test]
    fn test_stake_remove() {
        let mut db = tmp_db();
        db.put_stake(make_stake(1, 100_000));
        db.put_stake(make_stake(2, 200_000));
        assert_eq!(db.all_stakes().len(), 2);
        let removed = db.remove_stake(1).unwrap();
        assert_eq!(removed.staked_amount, 100_000);
        assert!(db.get_stake(1).is_none());
        assert_eq!(db.all_stakes().len(), 1);
    }

    #[test]
    fn test_stake_persistence_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        {
            let mut db = RocksDBStateDB::open(&path).unwrap();
            db.put_stake(make_stake(7, 777_000));
            db.put_stake(make_stake(8, 888_000));
        }
        let db = RocksDBStateDB::open(&path).unwrap();
        assert_eq!(db.all_stakes().len(), 2);
        assert_eq!(db.get_stake(7).unwrap().staked_amount, 777_000);
        assert_eq!(db.get_stake(8).unwrap().staked_amount, 888_000);
    }

    #[test]
    fn test_delegation_roundtrip() {
        let mut db = tmp_db();
        let d = make_delegation(0xAA, 5, 50_000);
        db.put_delegation(d.clone());
        let addr = [0xAAu8; 32];
        let loaded = db.get_delegation(&addr, 5).unwrap();
        assert_eq!(loaded.amount, 50_000);
    }

    #[test]
    fn test_delegation_remove() {
        let mut db = tmp_db();
        db.put_delegation(make_delegation(0x01, 1, 10_000));
        db.put_delegation(make_delegation(0x02, 1, 20_000));
        db.put_delegation(make_delegation(0x01, 2, 30_000));
        assert_eq!(db.all_delegations().len(), 3);
        let addr1 = [0x01u8; 32];
        db.remove_delegation(&addr1, 1);
        assert_eq!(db.all_delegations().len(), 2);
        assert!(db.get_delegation(&addr1, 1).is_none());
        assert!(db.get_delegation(&addr1, 2).is_some());
    }

    #[test]
    fn test_delegations_for_validator() {
        let mut db = tmp_db();
        db.put_delegation(make_delegation(0x01, 10, 1_000));
        db.put_delegation(make_delegation(0x02, 10, 2_000));
        db.put_delegation(make_delegation(0x03, 20, 3_000));
        let v10 = db.delegations_for_validator(10);
        assert_eq!(v10.len(), 2);
        let v20 = db.delegations_for_validator(20);
        assert_eq!(v20.len(), 1);
    }

    #[test]
    fn test_delegations_for_delegator() {
        let mut db = tmp_db();
        db.put_delegation(make_delegation(0x01, 1, 1_000));
        db.put_delegation(make_delegation(0x01, 2, 2_000));
        db.put_delegation(make_delegation(0x02, 1, 3_000));
        let addr1 = [0x01u8; 32];
        let d1 = db.delegations_for_delegator(&addr1);
        assert_eq!(d1.len(), 2);
        let addr2 = [0x02u8; 32];
        let d2 = db.delegations_for_delegator(&addr2);
        assert_eq!(d2.len(), 1);
    }

    #[test]
    fn test_delegation_persistence_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        {
            let mut db = RocksDBStateDB::open(&path).unwrap();
            db.put_delegation(make_delegation(0xAA, 42, 99_000));
        }
        let db = RocksDBStateDB::open(&path).unwrap();
        let addr = [0xAAu8; 32];
        assert_eq!(db.all_delegations().len(), 1);
        assert_eq!(db.get_delegation(&addr, 42).unwrap().amount, 99_000);
    }

    #[test]
    fn test_prune_before_height_persists_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();

        {
            let mut db = RocksDBStateDB::open(&path).unwrap();
            for epoch in [10u64, 50, 100] {
                let mut id = [0u8; 32];
                id[0] = epoch as u8;
                db.put_ghost(GhostRecord {
                    object_id: id,
                    owner: [0u8; 32],
                    evaporated_at: epoch,
                    data_hash: [0u8; 32],
                    original_data: None,
                    mmr_position: None,
                    original_half_life: None,
                });
            }
            let pruned = db.prune_before_height(100);
            assert_eq!(pruned, 2);
        }

        // Reopen — pruned ghosts must stay deleted on disk
        let db = RocksDBStateDB::open(&path).unwrap();
        assert_eq!(db.ghost_count(), 1);
        let mut id100 = [0u8; 32];
        id100[0] = 100;
        assert!(db.get_ghost(&id100).is_some());
    }
}
