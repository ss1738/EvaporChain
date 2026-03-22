use evaporchain_types::StateObject;
use std::collections::HashMap;

/// Trait for state database backends.
pub trait StateDB: Send + Sync {
    /// Retrieve a state object by its ID.
    fn get_object(&self, id: &[u8; 32]) -> Option<StateObject>;
    /// Store or update a state object.
    fn put_object(&mut self, obj: StateObject);
    /// Delete a state object by its ID.
    fn delete_object(&mut self, id: &[u8; 32]);
    /// Get the current state root hash.
    fn get_state_root(&self) -> [u8; 32];
}

/// In-memory state database for development and testing.
pub struct InMemoryStateDB {
    objects: HashMap<[u8; 32], StateObject>,
}

impl InMemoryStateDB {
    pub fn new() -> Self {
        Self {
            objects: HashMap::new(),
        }
    }
}

impl Default for InMemoryStateDB {
    fn default() -> Self {
        Self::new()
    }
}

impl StateDB for InMemoryStateDB {
    fn get_object(&self, id: &[u8; 32]) -> Option<StateObject> {
        self.objects.get(id).cloned()
    }

    fn put_object(&mut self, obj: StateObject) {
        self.objects.insert(obj.id, obj);
    }

    fn delete_object(&mut self, id: &[u8; 32]) {
        self.objects.remove(id);
    }

    fn get_state_root(&self) -> [u8; 32] {
        // Placeholder: return a hash of the number of objects
        let count = self.objects.len() as u64;
        let mut root = [0u8; 32];
        root[..8].copy_from_slice(&count.to_le_bytes());
        root
    }
}
