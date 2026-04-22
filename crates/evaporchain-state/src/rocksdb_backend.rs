//! RocksDB-backed state database with write-through cache.
//!
//! Maintains in-memory HashMaps for zero-overhead reads while persisting
//! every mutation to RocksDB. On startup, all data is loaded from disk
//! into the cache, so the node resumes exactly where it left off.

use crate::db::{build_energy_trie, trie_key_for_account, trie_key_for_object, trie_value_for_account, trie_value_for_object, StateDB};
use evaporchain_crypto::{EnergyVerkleTrie, TrieHealth};
use evaporchain_types::{Account, AccountAddress, GhostRecord, ObjectId, StateObject};
use rocksdb::{ColumnFamily, ColumnFamilyDescriptor, Options, DB};
use std::collections::{HashMap, HashSet};
use std::path::Path;

const CF_OBJECTS: &str = "objects";
const CF_GHOSTS: &str = "ghosts";
const CF_ACCOUNTS: &str = "accounts";
const CF_TRIE: &str = "trie";
const TRIE_SNAPSHOT_KEY: &[u8] = b"__energy_verkle_trie__";

/// RocksDB-backed state database with in-memory write-through cache.
pub struct RocksDBStateDB {
    db: DB,
    objects: HashMap<ObjectId, StateObject>,
    ghosts: HashMap<ObjectId, GhostRecord>,
    accounts: HashMap<AccountAddress, Account>,
    trie: EnergyVerkleTrie,
    dirty_objects: HashSet<ObjectId>,
    dirty_accounts: HashSet<AccountAddress>,
    // Privacy layer state (in-memory; RocksDB persistence in future pass)
    note_tree_root: [u8; 32],
    spent_nullifiers: std::collections::HashSet<[u8; 32]>,
    shielded_pool_balance: u64,
    note_count: u64,
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
        ];

        let db = DB::open_cf_descriptors(&opts, path, cf_descriptors)
            .map_err(|e| format!("Failed to open RocksDB: {}", e))?;

        // Load all data from disk into memory
        let mut objects = HashMap::new();
        let mut ghosts = HashMap::new();
        let mut accounts = HashMap::new();

        // Load objects
        let cf_obj = db.cf_handle(CF_OBJECTS)
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
        let cf_ghost = db.cf_handle(CF_GHOSTS)
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
                    eprintln!("  Warning: skipping unrecoverable ghost record {}", hex::encode(&id[..4]));
                }
            }
        }
        // Re-persist migrated ghosts in the current format and compact
        if ghost_migrated > 0 {
            let cf_g = db.cf_handle(CF_GHOSTS)
                .ok_or_else(|| format!("missing column family: {CF_GHOSTS}"))?;
            for ghost in ghosts.values() {
                let val = bincode::serialize(ghost).expect("serialize ghost");
                db.put_cf(cf_g, ghost.object_id, val).expect("migrate ghost to RocksDB");
            }
            // Force compaction so old SST entries are replaced
            db.compact_range_cf(cf_g, None::<&[u8]>, None::<&[u8]>);
            eprintln!("  Migrated and compacted {} ghost records", ghost_migrated);
        }

        // Load accounts
        let cf_acct = db.cf_handle(CF_ACCOUNTS)
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

        // Load persisted trie or rebuild from scratch
        let trie = {
            let cf_trie = db.cf_handle(CF_TRIE)
                .ok_or_else(|| format!("missing column family: {CF_TRIE}"))?;
            match db.get_cf(cf_trie, TRIE_SNAPSHOT_KEY) {
                Ok(Some(bytes)) => {
                    match EnergyVerkleTrie::from_bytes(&bytes) {
                        Ok(t) => {
                            println!("  RocksDB: loaded energy-verkle trie from disk ({} bytes)", bytes.len());
                            t
                        }
                        Err(e) => {
                            eprintln!("  Warning: trie snapshot corrupt ({}), rebuilding", e);
                            build_energy_trie(&objects, &accounts)
                        }
                    }
                }
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

        Ok(Self {
            db,
            objects,
            ghosts,
            accounts,
            trie,
            dirty_objects: HashSet::new(),
            dirty_accounts: HashSet::new(),
            note_tree_root: [0u8; 32],
            spent_nullifiers: std::collections::HashSet::new(),
            shielded_pool_balance: 0,
            note_count: 0,
        })
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
        let cf = self.cf(CF_OBJECTS);
        let value = bincode::serialize(obj).expect("serialize object");
        self.db.put_cf(cf, obj.id, value).expect("write object to RocksDB");
    }

    fn delete_object_disk(&self, id: &ObjectId) {
        let cf = self.cf(CF_OBJECTS);
        self.db.delete_cf(cf, id).expect("delete object from RocksDB");
    }

    fn persist_ghost(&self, ghost: &GhostRecord) {
        let cf = self.cf(CF_GHOSTS);
        let value = bincode::serialize(ghost).expect("serialize ghost");
        self.db.put_cf(cf, ghost.object_id, value).expect("write ghost to RocksDB");
    }

    fn delete_ghost_disk(&self, id: &ObjectId) {
        let cf = self.cf(CF_GHOSTS);
        self.db.delete_cf(cf, id).expect("delete ghost from RocksDB");
    }

    fn persist_account(&self, account: &Account) {
        let cf = self.cf(CF_ACCOUNTS);
        let value = bincode::serialize(account).expect("serialize account");
        self.db.put_cf(cf, account.address, value).expect("write account to RocksDB");
    }

    fn sync_dirty_to_trie(&mut self) {
        for id in self.dirty_objects.drain() {
            let key = trie_key_for_object(&id);
            if let Some(obj) = self.objects.get(&id) {
                self.trie.insert(key, trie_value_for_object(obj), obj.energy, obj.half_life, obj.last_refreshed);
            } else {
                self.trie.delete(&key);
            }
        }
        for addr in self.dirty_accounts.drain() {
            let key = trie_key_for_account(&addr);
            if let Some(acc) = self.accounts.get(&addr) {
                self.trie.insert(key, trie_value_for_account(acc), u64::MAX, u64::MAX, 0);
            }
        }
    }

    fn persist_trie(&self) {
        let cf = self.cf(CF_TRIE);
        let bytes = self.trie.to_bytes();
        self.db.put_cf(cf, TRIE_SNAPSHOT_KEY, bytes).expect("write trie snapshot to RocksDB");
    }
}

/// Attempt to deserialize a ghost record from legacy format (no mmr_position field).
fn deserialize_legacy_ghost(data: &[u8], id: &ObjectId) -> Result<GhostRecord, Box<bincode::ErrorKind>> {
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

    let evaporated_at = u64::from_le_bytes(
        data[offset..offset + 8]
            .try_into()
            .map_err(|_| Box::new(bincode::ErrorKind::Custom("invalid evaporated_at bytes".into())))?,
    );
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
    })
}

impl StateDB for RocksDBStateDB {
    fn get_object(&self, id: &ObjectId) -> Option<&StateObject> {
        self.objects.get(id)
    }

    fn get_object_mut(&mut self, id: &ObjectId) -> Option<&mut StateObject> {
        if self.objects.contains_key(id) {
            self.dirty_objects.insert(*id);
        }
        self.objects.get_mut(id)
    }

    fn put_object(&mut self, obj: StateObject) {
        self.persist_object(&obj);
        let key = trie_key_for_object(&obj.id);
        let value = trie_value_for_object(&obj);
        self.trie.insert(key, value, obj.energy, obj.half_life, obj.last_refreshed);
        self.dirty_objects.remove(&obj.id);
        self.objects.insert(obj.id, obj);
    }

    fn delete_object(&mut self, id: &ObjectId) -> Option<StateObject> {
        self.delete_object_disk(id);
        let key = trie_key_for_object(id);
        self.trie.delete(&key);
        self.dirty_objects.remove(id);
        self.objects.remove(id)
    }

    fn put_ghost(&mut self, record: GhostRecord) {
        self.persist_ghost(&record);
        self.ghosts.insert(record.object_id, record);
    }

    fn get_ghost(&self, id: &ObjectId) -> Option<&GhostRecord> {
        self.ghosts.get(id)
    }

    fn remove_ghost(&mut self, id: &ObjectId) -> Option<GhostRecord> {
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
            self.dirty_accounts.insert(*addr);
        }
        self.accounts.get_mut(addr)
    }

    fn put_account(&mut self, account: Account) {
        self.persist_account(&account);
        let key = trie_key_for_account(&account.address);
        let value = trie_value_for_account(&account);
        self.trie.insert(key, value, u64::MAX, u64::MAX, 0);
        self.dirty_accounts.remove(&account.address);
        self.accounts.insert(account.address, account);
    }

    fn get_or_create_account(&mut self, addr: &AccountAddress) -> &mut Account {
        if !self.accounts.contains_key(addr) {
            let account = Account {
                address: *addr,
                balance: 0,
                nonce: 0,
            };
            self.persist_account(&account);
            let key = trie_key_for_account(&account.address);
            let value = trie_value_for_account(&account);
            self.trie.insert(key, value, u64::MAX, u64::MAX, 0);
            self.accounts.insert(*addr, account);
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
    }

    fn get_note_tree_root(&self) -> [u8; 32] {
        self.note_tree_root
    }

    fn spend_nullifier(&mut self, nullifier: &[u8; 32]) -> bool {
        self.spent_nullifiers.insert(*nullifier)
    }

    fn is_nullifier_spent(&self, nullifier: &[u8; 32]) -> bool {
        self.spent_nullifiers.contains(nullifier)
    }

    fn nullifier_count(&self) -> usize {
        self.spent_nullifiers.len()
    }

    fn put_shielded_pool_balance(&mut self, balance: u64) {
        self.shielded_pool_balance = balance;
    }

    fn get_shielded_pool_balance(&self) -> u64 {
        self.shielded_pool_balance
    }

    fn put_note_count(&mut self, count: u64) {
        self.note_count = count;
    }

    fn get_note_count(&self) -> u64 {
        self.note_count
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
}

/// Flush account changes back to RocksDB after mutable borrows.
/// Call this after any block execution that modifies accounts via get_account_mut()
/// or get_or_create_account() followed by balance/nonce changes.
impl RocksDBStateDB {
    pub fn flush_accounts(&mut self) {
        let cf = self.cf(CF_ACCOUNTS);
        for (_, account) in &self.accounts {
            let value = bincode::serialize(account).expect("serialize account");
            self.db.put_cf(cf, account.address, value).expect("flush account to RocksDB");
        }
        self.sync_dirty_to_trie();
        self.persist_trie();
    }

    pub fn flush_objects(&mut self) {
        let cf = self.cf(CF_OBJECTS);
        for (_, obj) in &self.objects {
            let value = bincode::serialize(obj).expect("serialize object");
            self.db.put_cf(cf, obj.id, value).expect("flush object to RocksDB");
        }
        self.sync_dirty_to_trie();
        self.persist_trie();
    }
}
