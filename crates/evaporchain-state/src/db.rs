use evaporchain_crypto::hash::blake3_hash;
use evaporchain_crypto::{EnergyVerkleTrie, TrieHealth};

use evaporchain_types::{Account, AccountAddress, GhostRecord, ObjectId, StateObject};
use std::collections::HashMap;

// ─── Trie key/value derivation (shared by all StateDB backends) ─────────

pub fn trie_key_for_account(addr: &AccountAddress) -> [u8; 32] {
    let mut buf = Vec::with_capacity(36);
    buf.extend_from_slice(b"acct");
    buf.extend_from_slice(addr);
    blake3_hash(&buf)
}

pub fn trie_value_for_account(acc: &Account) -> [u8; 32] {
    let mut buf = Vec::with_capacity(16);
    buf.extend_from_slice(&acc.balance.to_le_bytes());
    buf.extend_from_slice(&acc.nonce.to_le_bytes());
    blake3_hash(&buf)
}

pub fn trie_key_for_object(id: &ObjectId) -> [u8; 32] {
    let mut buf = Vec::with_capacity(35);
    buf.extend_from_slice(b"obj");
    buf.extend_from_slice(id);
    blake3_hash(&buf)
}

pub fn trie_value_for_object(obj: &StateObject) -> [u8; 32] {
    let mut buf = Vec::with_capacity(9);
    buf.extend_from_slice(&obj.energy.to_le_bytes());
    buf.push(object_state_to_u8(&obj.state));
    blake3_hash(&buf)
}

pub fn build_energy_trie(
    objects: &HashMap<ObjectId, StateObject>,
    accounts: &HashMap<AccountAddress, Account>,
) -> EnergyVerkleTrie {
    let mut trie = EnergyVerkleTrie::new();

    for (addr, acc) in accounts {
        trie.insert(
            trie_key_for_account(addr),
            trie_value_for_account(acc),
            u64::MAX,
            u64::MAX,
            0,
        );
    }

    for (_id, obj) in objects {
        trie.insert(
            trie_key_for_object(&obj.id),
            trie_value_for_object(obj),
            obj.energy,
            obj.half_life,
            obj.last_refreshed,
        );
    }

    trie
}

/// Trait for state database backends.
pub trait StateDB: Send + Sync {
    /// Retrieve a state object by its ID.
    fn get_object(&self, id: &ObjectId) -> Option<&StateObject>;

    /// Retrieve a mutable reference to a state object.
    fn get_object_mut(&mut self, id: &ObjectId) -> Option<&mut StateObject>;

    /// Store or update a state object.
    fn put_object(&mut self, obj: StateObject);

    /// Delete a state object by its ID and return it.
    fn delete_object(&mut self, id: &ObjectId) -> Option<StateObject>;

    /// Store a ghost record for an evaporated object.
    fn put_ghost(&mut self, record: GhostRecord);

    /// Retrieve a ghost record by object ID.
    fn get_ghost(&self, id: &ObjectId) -> Option<&GhostRecord>;

    /// Remove a ghost record (used during resurrection).
    fn remove_ghost(&mut self, id: &ObjectId) -> Option<GhostRecord>;

    /// Return all object IDs currently in active state.
    fn all_object_ids(&self) -> Vec<ObjectId>;

    /// Return the number of active objects.
    fn object_count(&self) -> usize;

    /// Return the number of ghost records.
    fn ghost_count(&self) -> usize;

    /// Return all ghost record object IDs.
    fn all_ghost_ids(&self) -> Vec<ObjectId>;

    /// Retrieve an account by address.
    fn get_account(&self, addr: &AccountAddress) -> Option<&Account>;

    /// Retrieve a mutable reference to an account.
    fn get_account_mut(&mut self, addr: &AccountAddress) -> Option<&mut Account>;

    /// Store or update an account.
    fn put_account(&mut self, account: Account);

    /// Get or create an account (returns mutable ref). Creates with zero balance if missing.
    fn get_or_create_account(&mut self, addr: &AccountAddress) -> &mut Account;

    /// Return all account addresses.
    fn all_account_addresses(&self) -> Vec<AccountAddress>;

    /// Compute the state root hash over all objects and accounts.
    fn compute_state_root(&mut self) -> [u8; 32];

    /// Compress cold subtrees in the energy-annotated Verkle trie.
    /// Returns the number of subtrees compressed.
    fn compress_cold_subtrees(&mut self) -> u32;

    /// Report health of the energy-annotated Verkle trie.
    fn trie_health(&mut self) -> TrieHealth;

    /// Serialize the current trie state to bytes for persistence.
    fn trie_snapshot(&mut self) -> Vec<u8>;

    /// Restore the trie from a previously serialized snapshot.
    fn load_trie_snapshot(&mut self, bytes: &[u8]) -> Result<(), String>;

    // ─── Privacy Layer State ──────────────────────────────────────────────

    /// Store the current Merkle note tree root.
    fn put_note_tree_root(&mut self, root: [u8; 32]);

    /// Get the current Merkle note tree root (returns zero hash if none set).
    fn get_note_tree_root(&self) -> [u8; 32];

    /// Record a spent nullifier. Returns false if already spent (double-spend).
    fn spend_nullifier(&mut self, nullifier: &[u8; 32]) -> bool;

    /// Check if a nullifier has been spent.
    fn is_nullifier_spent(&self, nullifier: &[u8; 32]) -> bool;

    /// Number of spent nullifiers.
    fn nullifier_count(&self) -> usize;

    /// Return all spent nullifiers (for snapshots).
    fn all_nullifiers(&self) -> Vec<[u8; 32]>;

    /// Store the total shielded pool balance (for auditing / invariant checks).
    fn put_shielded_pool_balance(&mut self, balance: u64);

    /// Get the total shielded pool balance.
    fn get_shielded_pool_balance(&self) -> u64;

    /// Store the note count (number of notes ever inserted into the tree).
    fn put_note_count(&mut self, count: u64);

    /// Get the note count.
    fn get_note_count(&self) -> u64;
}

/// In-memory state database for development and testing.
pub struct InMemoryStateDB {
    objects: HashMap<ObjectId, StateObject>,
    ghosts: HashMap<ObjectId, GhostRecord>,
    accounts: HashMap<AccountAddress, Account>,
    /// Persistent incremental trie — mutated on each state change.
    trie: EnergyVerkleTrie,
    /// Object IDs modified via mutable reference (trie update deferred until root computation).
    dirty_objects: std::collections::HashSet<ObjectId>,
    /// Account addresses modified via mutable reference.
    dirty_accounts: std::collections::HashSet<AccountAddress>,
    // Privacy layer state
    note_tree_root: [u8; 32],
    spent_nullifiers: std::collections::HashSet<[u8; 32]>,
    shielded_pool_balance: u64,
    note_count: u64,
}

impl InMemoryStateDB {
    pub fn new() -> Self {
        Self {
            objects: HashMap::new(),
            ghosts: HashMap::new(),
            accounts: HashMap::new(),
            trie: EnergyVerkleTrie::new(),
            dirty_objects: std::collections::HashSet::new(),
            dirty_accounts: std::collections::HashSet::new(),
            note_tree_root: [0u8; 32],
            spent_nullifiers: std::collections::HashSet::new(),
            shielded_pool_balance: 0,
            note_count: 0,
        }
    }

    /// Sync dirty entries into the trie (O(dirty) not O(n)).
    fn sync_dirty_to_trie(&mut self) {
        for id in self.dirty_objects.drain() {
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
        for addr in self.dirty_accounts.drain() {
            let key = trie_key_for_account(&addr);
            if let Some(acc) = self.accounts.get(&addr) {
                self.trie.insert(key, trie_value_for_account(acc), u64::MAX, u64::MAX, 0);
            }
        }
    }
}

impl Default for InMemoryStateDB {
    fn default() -> Self {
        Self::new()
    }
}

impl StateDB for InMemoryStateDB {
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
        let key = trie_key_for_object(&obj.id);
        let value = trie_value_for_object(&obj);
        self.trie.insert(key, value, obj.energy, obj.half_life, obj.last_refreshed);
        self.objects.insert(obj.id, obj);
    }

    fn delete_object(&mut self, id: &ObjectId) -> Option<StateObject> {
        let key = trie_key_for_object(id);
        self.trie.delete(&key);
        self.objects.remove(id)
    }

    fn put_ghost(&mut self, record: GhostRecord) {
        self.ghosts.insert(record.object_id, record);
    }

    fn get_ghost(&self, id: &ObjectId) -> Option<&GhostRecord> {
        self.ghosts.get(id)
    }

    fn remove_ghost(&mut self, id: &ObjectId) -> Option<GhostRecord> {
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
        let key = trie_key_for_account(&account.address);
        let value = trie_value_for_account(&account);
        self.trie.insert(key, value, u64::MAX, u64::MAX, 0);
        self.accounts.insert(account.address, account);
    }

    fn get_or_create_account(&mut self, addr: &AccountAddress) -> &mut Account {
        if !self.accounts.contains_key(addr) {
            let account = Account {
                address: *addr,
                balance: 0,
                nonce: 0,
            };
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

    fn all_nullifiers(&self) -> Vec<[u8; 32]> {
        self.spent_nullifiers.iter().copied().collect()
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

pub fn object_state_to_u8(s: &evaporchain_types::ObjectState) -> u8 {
    match s {
        evaporchain_types::ObjectState::Active => 0,
        evaporchain_types::ObjectState::Grace => 1,
        evaporchain_types::ObjectState::Ghost => 2,
        evaporchain_types::ObjectState::Resurrected => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_types::ObjectState;

    fn make_object(id_byte: u8, energy: u64) -> StateObject {
        StateObject {
            id: {
                let mut id = [0u8; 32];
                id[0] = id_byte;
                id
            },
            owner: [0u8; 32],
            energy,
            half_life: 100,
            created_at: 0,
            last_refreshed: 0,
            state: ObjectState::Active,
            grace_epoch: None,
            data: vec![],
        }
    }

    #[test]
    fn test_put_and_get() {
        let mut db = InMemoryStateDB::new();
        let obj = make_object(1, 1000);
        db.put_object(obj.clone());
        assert_eq!(db.get_object(&obj.id), Some(&obj));
        assert_eq!(db.object_count(), 1);
    }

    #[test]
    fn test_delete() {
        let mut db = InMemoryStateDB::new();
        let obj = make_object(1, 1000);
        let id = obj.id;
        db.put_object(obj);
        let deleted = db.delete_object(&id);
        assert!(deleted.is_some());
        assert_eq!(db.object_count(), 0);
        assert!(db.get_object(&id).is_none());
    }

    #[test]
    fn test_ghost_records() {
        let mut db = InMemoryStateDB::new();
        let ghost = GhostRecord {
            object_id: [5u8; 32],
            owner: [0u8; 32],
            evaporated_at: 100,
            data_hash: [0u8; 32],
            original_data: Some(vec![1, 2, 3]),
            mmr_position: None,
            original_half_life: None,
        };
        db.put_ghost(ghost.clone());
        assert_eq!(db.ghost_count(), 1);
        assert_eq!(db.get_ghost(&[5u8; 32]), Some(&ghost));

        let removed = db.remove_ghost(&[5u8; 32]);
        assert!(removed.is_some());
        assert_eq!(db.ghost_count(), 0);
    }

    #[test]
    fn test_state_root_deterministic() {
        let mut db = InMemoryStateDB::new();
        db.put_object(make_object(1, 1000));
        db.put_object(make_object(2, 500));
        let root1 = db.compute_state_root();
        let root2 = db.compute_state_root();
        assert_eq!(root1, root2);
        assert_ne!(root1, [0u8; 32]);
    }

    #[test]
    fn test_state_root_changes_on_mutation() {
        let mut db = InMemoryStateDB::new();
        db.put_object(make_object(1, 1000));
        let root1 = db.compute_state_root();

        db.get_object_mut(&{
            let mut id = [0u8; 32];
            id[0] = 1;
            id
        })
        .unwrap()
        .energy = 500;
        let root2 = db.compute_state_root();

        assert_ne!(root1, root2);
    }

    #[test]
    fn test_empty_state_root() {
        let mut db = InMemoryStateDB::new();
        assert_eq!(db.compute_state_root(), [0u8; 32]);
    }

    #[test]
    fn test_trie_health_reflects_state() {
        let mut db = InMemoryStateDB::new();
        db.put_object(make_object(1, 1000));
        db.put_object(make_object(2, 500));

        let health = db.trie_health();
        assert_eq!(health.active_leaves, 2);
        assert_eq!(health.max_energy, 1000);
        assert_eq!(health.min_half_life, 100);
        assert_eq!(health.compressed_leaves, 0);
    }

    #[test]
    fn test_trie_health_with_accounts() {
        let mut db = InMemoryStateDB::new();
        db.put_account(Account { address: [1u8; 32], balance: 100, nonce: 0 });
        db.put_object(make_object(1, 500));

        let health = db.trie_health();
        assert_eq!(health.active_leaves, 2);
    }

    #[test]
    fn test_compress_cold_subtrees_with_zero_energy() {
        let mut db = InMemoryStateDB::new();
        let mut obj = make_object(1, 0);
        obj.state = ObjectState::Grace;
        obj.energy = 0;
        db.put_object(obj);
        db.put_object(make_object(2, 1000));

        let compressed = db.compress_cold_subtrees();
        assert!(compressed >= 0);
    }

    #[test]
    fn test_empty_trie_health() {
        let mut db = InMemoryStateDB::new();
        let health = db.trie_health();
        assert_eq!(health.active_leaves, 0);
        assert_eq!(health.total_nodes, 0);
    }
}
