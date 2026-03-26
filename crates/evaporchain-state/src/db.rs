use evaporchain_crypto::hash::blake3_hash;
use evaporchain_crypto::VerkleTrie;

use evaporchain_types::{Account, AccountAddress, GhostRecord, ObjectId, StateObject};
use std::collections::HashMap;

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
    fn compute_state_root(&self) -> [u8; 32];
}

/// In-memory state database for development and testing.
pub struct InMemoryStateDB {
    objects: HashMap<ObjectId, StateObject>,
    ghosts: HashMap<ObjectId, GhostRecord>,
    accounts: HashMap<AccountAddress, Account>,
}

impl InMemoryStateDB {
    pub fn new() -> Self {
        Self {
            objects: HashMap::new(),
            ghosts: HashMap::new(),
            accounts: HashMap::new(),
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
        self.objects.get_mut(id)
    }

    fn put_object(&mut self, obj: StateObject) {
        self.objects.insert(obj.id, obj);
    }

    fn delete_object(&mut self, id: &ObjectId) -> Option<StateObject> {
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
        self.accounts.get_mut(addr)
    }

    fn put_account(&mut self, account: Account) {
        self.accounts.insert(account.address, account);
    }

    fn get_or_create_account(&mut self, addr: &AccountAddress) -> &mut Account {
        self.accounts.entry(*addr).or_insert_with(|| Account {
            address: *addr,
            balance: 0,
            nonce: 0,
        })
    }

    fn all_account_addresses(&self) -> Vec<AccountAddress> {
        self.accounts.keys().copied().collect()
    }

    fn compute_state_root(&self) -> [u8; 32] {
        if self.objects.is_empty() && self.accounts.is_empty() {
            return [0u8; 32];
        }

        let mut trie = VerkleTrie::new();

        // Insert accounts into trie: key = blake3("acct" || address), value = hash(balance || nonce)
        for (addr, acc) in &self.accounts {
            let mut key_input = Vec::with_capacity(36);
            key_input.extend_from_slice(b"acct");
            key_input.extend_from_slice(addr);
            let key = blake3_hash(&key_input);

            let mut val_input = Vec::with_capacity(16);
            val_input.extend_from_slice(&acc.balance.to_le_bytes());
            val_input.extend_from_slice(&acc.nonce.to_le_bytes());
            let value = blake3_hash(&val_input);

            trie.insert(key, value);
        }

        // Insert objects into trie: key = blake3("obj" || id), value = hash(energy || state)
        for (id, obj) in &self.objects {
            let mut key_input = Vec::with_capacity(35);
            key_input.extend_from_slice(b"obj");
            key_input.extend_from_slice(id);
            let key = blake3_hash(&key_input);

            let mut val_input = Vec::with_capacity(9);
            val_input.extend_from_slice(&obj.energy.to_le_bytes());
            val_input.push(object_state_to_u8(&obj.state));
            let value = blake3_hash(&val_input);

            trie.insert(key, value);
        }

        trie.root()
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
            original_data: vec![1, 2, 3],
            mmr_position: None,
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
        let db = InMemoryStateDB::new();
        assert_eq!(db.compute_state_root(), [0u8; 32]);
    }
}
