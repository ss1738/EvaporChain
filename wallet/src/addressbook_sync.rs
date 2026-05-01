// wallet/src/addressbook_sync.rs — Cross-device address book sync
//
// Vector-clock based conflict resolution for syncing contacts
// across multiple wallet instances. Supports merge, diff, and
// export/import for manual sync via file.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SyncError {
    #[error("contact not found: {0}")]
    NotFound(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("invalid device id: {0}")]
    InvalidDevice(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("json error: {0}")]
    Json(String),
}

// ── Vector clock ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct VectorClock {
    pub clocks: HashMap<String, u64>,
}

impl VectorClock {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn increment(&mut self, device_id: &str) {
        let counter = self.clocks.entry(device_id.to_string()).or_insert(0);
        *counter += 1;
    }

    pub fn get(&self, device_id: &str) -> u64 {
        self.clocks.get(device_id).copied().unwrap_or(0)
    }

    /// Merge two vector clocks (take max of each component)
    pub fn merge(&mut self, other: &VectorClock) {
        for (device, &count) in &other.clocks {
            let entry = self.clocks.entry(device.clone()).or_insert(0);
            *entry = (*entry).max(count);
        }
    }

    /// Check if self dominates other (all components >=)
    pub fn dominates(&self, other: &VectorClock) -> bool {
        for (device, &count) in &other.clocks {
            if self.get(device) < count {
                return false;
            }
        }
        true
    }

    /// Check if clocks are concurrent (neither dominates)
    pub fn is_concurrent(&self, other: &VectorClock) -> bool {
        !self.dominates(other) && !other.dominates(self)
    }
}

// ── Synced contact ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncedContact {
    pub address: String,
    pub name: String,
    pub notes: String,
    pub tags: Vec<String>,
    pub updated_at: String,
    pub updated_by: String,
    pub clock: VectorClock,
    pub deleted: bool,
}

impl SyncedContact {
    pub fn new(address: &str, name: &str, device_id: &str) -> Self {
        let mut clock = VectorClock::new();
        clock.increment(device_id);
        Self {
            address: address.to_string(),
            name: name.to_string(),
            notes: String::new(),
            tags: Vec::new(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            updated_by: device_id.to_string(),
            clock,
            deleted: false,
        }
    }

    pub fn update(&mut self, name: &str, device_id: &str) {
        self.name = name.to_string();
        self.updated_at = chrono::Utc::now().to_rfc3339();
        self.updated_by = device_id.to_string();
        self.clock.increment(device_id);
    }

    pub fn soft_delete(&mut self, device_id: &str) {
        self.deleted = true;
        self.updated_at = chrono::Utc::now().to_rfc3339();
        self.updated_by = device_id.to_string();
        self.clock.increment(device_id);
    }
}

// ── Conflict resolution ───────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConflictResolution {
    KeepLocal,
    KeepRemote,
    KeepNewer,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConflict {
    pub address: String,
    pub local: SyncedContact,
    pub remote: SyncedContact,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncResult {
    pub added: usize,
    pub updated: usize,
    pub deleted: usize,
    pub conflicts: Vec<SyncConflict>,
}

// ── Sync store ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncableAddressBook {
    pub device_id: String,
    pub contacts: HashMap<String, SyncedContact>,
    pub last_sync: Option<String>,
    pub sync_count: u64,
}

impl SyncableAddressBook {
    pub fn new(device_id: &str) -> Self {
        Self {
            device_id: device_id.to_string(),
            contacts: HashMap::new(),
            last_sync: None,
            sync_count: 0,
        }
    }

    pub fn add(&mut self, address: &str, name: &str) -> Result<(), SyncError> {
        if self.contacts.contains_key(address) {
            return Err(SyncError::Conflict(format!(
                "contact already exists: {}",
                address
            )));
        }
        self.contacts.insert(
            address.to_string(),
            SyncedContact::new(address, name, &self.device_id),
        );
        Ok(())
    }

    pub fn update(&mut self, address: &str, name: &str) -> Result<(), SyncError> {
        let contact = self
            .contacts
            .get_mut(address)
            .ok_or_else(|| SyncError::NotFound(address.into()))?;
        contact.update(name, &self.device_id);
        Ok(())
    }

    pub fn delete(&mut self, address: &str) -> Result<(), SyncError> {
        let contact = self
            .contacts
            .get_mut(address)
            .ok_or_else(|| SyncError::NotFound(address.into()))?;
        contact.soft_delete(&self.device_id);
        Ok(())
    }

    pub fn get(&self, address: &str) -> Option<&SyncedContact> {
        self.contacts.get(address).filter(|c| !c.deleted)
    }

    pub fn list(&self) -> Vec<&SyncedContact> {
        self.contacts.values().filter(|c| !c.deleted).collect()
    }

    pub fn list_all(&self) -> Vec<&SyncedContact> {
        self.contacts.values().collect()
    }

    /// Merge a remote address book into this one
    pub fn merge(
        &mut self,
        remote: &SyncableAddressBook,
        resolution: ConflictResolution,
    ) -> SyncResult {
        let mut result = SyncResult {
            added: 0,
            updated: 0,
            deleted: 0,
            conflicts: Vec::new(),
        };

        for (addr, remote_contact) in &remote.contacts {
            match self.contacts.get(addr) {
                None => {
                    // New contact from remote
                    self.contacts.insert(addr.clone(), remote_contact.clone());
                    if remote_contact.deleted {
                        result.deleted += 1;
                    } else {
                        result.added += 1;
                    }
                }
                Some(local_contact) => {
                    if local_contact.clock.dominates(&remote_contact.clock) {
                        // Local is newer — keep local
                        continue;
                    } else if remote_contact.clock.dominates(&local_contact.clock) {
                        // Remote is newer — take remote
                        self.contacts.insert(addr.clone(), remote_contact.clone());
                        result.updated += 1;
                    } else {
                        // Concurrent changes — conflict
                        match resolution {
                            ConflictResolution::KeepLocal => {} // Do nothing
                            ConflictResolution::KeepRemote => {
                                self.contacts.insert(addr.clone(), remote_contact.clone());
                                result.updated += 1;
                            }
                            ConflictResolution::KeepNewer => {
                                if remote_contact.updated_at > local_contact.updated_at {
                                    self.contacts.insert(addr.clone(), remote_contact.clone());
                                    result.updated += 1;
                                }
                            }
                            ConflictResolution::Manual => {
                                result.conflicts.push(SyncConflict {
                                    address: addr.clone(),
                                    local: local_contact.clone(),
                                    remote: remote_contact.clone(),
                                });
                            }
                        }
                    }
                }
            }
        }

        // Merge vector clocks for all contacts
        for (addr, contact) in self.contacts.iter_mut() {
            if let Some(remote_c) = remote.contacts.get(addr) {
                contact.clock.merge(&remote_c.clock);
            }
        }

        self.sync_count += 1;
        self.last_sync = Some(chrono::Utc::now().to_rfc3339());

        result
    }

    /// Export for sync (serialized)
    pub fn export_for_sync(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Diff: what changed since a given vector clock snapshot
    pub fn changes_since(&self, since: &HashMap<String, VectorClock>) -> Vec<&SyncedContact> {
        self.contacts
            .values()
            .filter(|c| {
                match since.get(&c.address) {
                    None => true, // New contact
                    Some(old_clock) => !old_clock.dominates(&c.clock),
                }
            })
            .collect()
    }

    pub fn save(&self, path: &Path) -> Result<(), SyncError> {
        let json =
            serde_json::to_string_pretty(self).map_err(|e| SyncError::Json(e.to_string()))?;
        std::fs::write(path, json).map_err(|e| SyncError::Io(e.to_string()))
    }

    pub fn load(path: &Path) -> Result<Self, SyncError> {
        let data = std::fs::read_to_string(path).map_err(|e| SyncError::Io(e.to_string()))?;
        serde_json::from_str(&data).map_err(|e| SyncError::Json(e.to_string()))
    }
}

// ── Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vector_clock_increment() {
        let mut vc = VectorClock::new();
        vc.increment("dev1");
        vc.increment("dev1");
        assert_eq!(vc.get("dev1"), 2);
        assert_eq!(vc.get("dev2"), 0);
    }

    #[test]
    fn test_vector_clock_merge() {
        let mut vc1 = VectorClock::new();
        vc1.increment("dev1");
        vc1.increment("dev1");

        let mut vc2 = VectorClock::new();
        vc2.increment("dev2");

        vc1.merge(&vc2);
        assert_eq!(vc1.get("dev1"), 2);
        assert_eq!(vc1.get("dev2"), 1);
    }

    #[test]
    fn test_vector_clock_dominates() {
        let mut vc1 = VectorClock::new();
        vc1.increment("dev1");
        vc1.increment("dev1");

        let mut vc2 = VectorClock::new();
        vc2.increment("dev1");

        assert!(vc1.dominates(&vc2));
        assert!(!vc2.dominates(&vc1));
    }

    #[test]
    fn test_vector_clock_concurrent() {
        let mut vc1 = VectorClock::new();
        vc1.increment("dev1");

        let mut vc2 = VectorClock::new();
        vc2.increment("dev2");

        assert!(vc1.is_concurrent(&vc2));
    }

    #[test]
    fn test_synced_contact_new() {
        let c = SyncedContact::new("evap1abc", "Alice", "dev1");
        assert_eq!(c.name, "Alice");
        assert_eq!(c.clock.get("dev1"), 1);
        assert!(!c.deleted);
    }

    #[test]
    fn test_synced_contact_update() {
        let mut c = SyncedContact::new("evap1abc", "Alice", "dev1");
        c.update("Alice Smith", "dev1");
        assert_eq!(c.name, "Alice Smith");
        assert_eq!(c.clock.get("dev1"), 2);
    }

    #[test]
    fn test_synced_contact_soft_delete() {
        let mut c = SyncedContact::new("evap1abc", "Alice", "dev1");
        c.soft_delete("dev1");
        assert!(c.deleted);
    }

    #[test]
    fn test_addressbook_add() {
        let mut book = SyncableAddressBook::new("dev1");
        book.add("evap1abc", "Alice").unwrap();
        assert_eq!(book.list().len(), 1);
    }

    #[test]
    fn test_addressbook_add_duplicate() {
        let mut book = SyncableAddressBook::new("dev1");
        book.add("evap1abc", "Alice").unwrap();
        assert!(book.add("evap1abc", "Alice2").is_err());
    }

    #[test]
    fn test_addressbook_update() {
        let mut book = SyncableAddressBook::new("dev1");
        book.add("evap1abc", "Alice").unwrap();
        book.update("evap1abc", "Alice Smith").unwrap();
        assert_eq!(book.get("evap1abc").unwrap().name, "Alice Smith");
    }

    #[test]
    fn test_addressbook_delete() {
        let mut book = SyncableAddressBook::new("dev1");
        book.add("evap1abc", "Alice").unwrap();
        book.delete("evap1abc").unwrap();
        assert!(book.get("evap1abc").is_none()); // Soft-deleted, hidden from get
        assert_eq!(book.list_all().len(), 1); // Still in store
    }

    #[test]
    fn test_merge_new_contact() {
        let mut local = SyncableAddressBook::new("dev1");
        local.add("evap1a", "Alice").unwrap();

        let mut remote = SyncableAddressBook::new("dev2");
        remote.add("evap1b", "Bob").unwrap();

        let result = local.merge(&remote, ConflictResolution::KeepNewer);
        assert_eq!(result.added, 1);
        assert_eq!(local.list().len(), 2);
    }

    #[test]
    fn test_merge_remote_newer() {
        let mut local = SyncableAddressBook::new("dev1");
        local.add("evap1a", "Alice").unwrap();

        let mut remote = SyncableAddressBook::new("dev2");
        remote.add("evap1a", "Alice Updated").unwrap();
        // Remote has clock dev2=1, local has dev1=1 — concurrent

        let result = local.merge(&remote, ConflictResolution::KeepRemote);
        assert_eq!(result.updated, 1);
        assert_eq!(local.get("evap1a").unwrap().name, "Alice Updated");
    }

    #[test]
    fn test_merge_keep_local_on_conflict() {
        let mut local = SyncableAddressBook::new("dev1");
        local.add("evap1a", "Local Alice").unwrap();

        let mut remote = SyncableAddressBook::new("dev2");
        remote.add("evap1a", "Remote Alice").unwrap();

        let result = local.merge(&remote, ConflictResolution::KeepLocal);
        assert_eq!(result.conflicts.len(), 0);
        assert_eq!(local.get("evap1a").unwrap().name, "Local Alice");
    }

    #[test]
    fn test_merge_manual_conflicts() {
        let mut local = SyncableAddressBook::new("dev1");
        local.add("evap1a", "Local").unwrap();

        let mut remote = SyncableAddressBook::new("dev2");
        remote.add("evap1a", "Remote").unwrap();

        let result = local.merge(&remote, ConflictResolution::Manual);
        assert_eq!(result.conflicts.len(), 1);
    }

    #[test]
    fn test_merge_remote_dominates() {
        let mut local = SyncableAddressBook::new("dev1");
        local.add("evap1a", "Alice").unwrap();

        // Create remote that dominates local
        let mut remote = SyncableAddressBook::new("dev1");
        remote.add("evap1a", "Alice V2").unwrap();
        remote.update("evap1a", "Alice V3").unwrap(); // dev1=2 > local dev1=1

        let result = local.merge(&remote, ConflictResolution::Manual);
        assert_eq!(result.updated, 1);
        assert_eq!(result.conflicts.len(), 0);
    }

    #[test]
    fn test_changes_since_empty() {
        let book = SyncableAddressBook::new("dev1");
        let since = HashMap::new();
        assert_eq!(book.changes_since(&since).len(), 0);
    }

    #[test]
    fn test_changes_since_new_contact() {
        let mut book = SyncableAddressBook::new("dev1");
        book.add("evap1a", "Alice").unwrap();
        let since = HashMap::new();
        assert_eq!(book.changes_since(&since).len(), 1);
    }

    #[test]
    fn test_export_for_sync() {
        let mut book = SyncableAddressBook::new("dev1");
        book.add("evap1a", "Alice").unwrap();
        let json = book.export_for_sync();
        assert!(json.contains("evap1a"));
        assert!(json.contains("Alice"));
    }

    #[test]
    fn test_save_load() {
        let path = std::env::temp_dir().join(format!("evap_sync_{}.json", std::process::id()));
        let mut book = SyncableAddressBook::new("dev1");
        book.add("evap1a", "Alice").unwrap();
        book.save(&path).unwrap();
        let loaded = SyncableAddressBook::load(&path).unwrap();
        assert_eq!(loaded.list().len(), 1);
        assert_eq!(loaded.device_id, "dev1");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_sync_count() {
        let mut local = SyncableAddressBook::new("dev1");
        let remote = SyncableAddressBook::new("dev2");
        local.merge(&remote, ConflictResolution::KeepNewer);
        assert_eq!(local.sync_count, 1);
        assert!(local.last_sync.is_some());
    }
}
