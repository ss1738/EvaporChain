//! ENS-style name resolution service for EvaporChain.
//!
//! Register human-readable names (e.g. "alice.evap"), attach records,
//! resolve forward (name → address) and reverse (address → name).
//! Persisted to JSON on disk.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum NameServiceError {
    #[error("name already exists: {0}")]
    AlreadyExists(String),
    #[error("name not found: {0}")]
    NotFound(String),
    #[error("name is reserved: {0}")]
    Reserved(String),
    #[error("not owner: {0}")]
    NotOwner(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse error: {0}")]
    Parse(#[from] serde_json::Error),
}

// ──────────────────────────── Enums ─────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NameStatus {
    Active,
    Expired,
    Reserved,
    Pending,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecordType {
    Address,
    ContentHash,
    Text,
    Avatar,
    Email,
    Url,
    Github,
    Twitter,
    Custom(String),
}

// ──────────────────────────── NameRecord ─────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NameRecord {
    pub key: RecordType,
    pub value: String,
}

impl NameRecord {
    pub fn new(key: RecordType, value: String) -> Self {
        Self { key, value }
    }
}

// ──────────────────────────── RegisteredName ────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisteredName {
    pub name: String,
    pub owner: String,
    pub resolver: Option<String>,
    pub status: NameStatus,
    pub registered_at: String,
    pub expires_at: String,
    pub records: Vec<NameRecord>,
    pub subnames: Vec<String>,
    pub transfer_history: Vec<(String, String, String)>,
    pub metadata: HashMap<String, String>,
}

impl RegisteredName {
    pub fn new(name: &str, owner: &str, expires_at: &str) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            name: name.to_string(),
            owner: owner.to_string(),
            resolver: None,
            status: NameStatus::Active,
            registered_at: now,
            expires_at: expires_at.to_string(),
            records: Vec::new(),
            subnames: Vec::new(),
            transfer_history: Vec::new(),
            metadata: HashMap::new(),
        }
    }

    pub fn with_resolver(mut self, resolver: &str) -> Self {
        self.resolver = Some(resolver.to_string());
        self
    }

    pub fn add_record(&mut self, record: NameRecord) {
        self.records.push(record);
    }

    pub fn get_record(&self, key: &RecordType) -> Option<&str> {
        self.records
            .iter()
            .find(|r| &r.key == key)
            .map(|r| r.value.as_str())
    }

    pub fn remove_record(&mut self, key: &RecordType) -> bool {
        let before = self.records.len();
        self.records.retain(|r| &r.key != key);
        self.records.len() < before
    }

    pub fn add_subname(&mut self, subname: &str) {
        self.subnames.push(subname.to_string());
    }

    pub fn is_expired(&self) -> bool {
        if let Ok(exp) = chrono::DateTime::parse_from_rfc3339(&self.expires_at) {
            exp < chrono::Utc::now()
        } else {
            false
        }
    }

    pub fn is_active(&self) -> bool {
        self.status == NameStatus::Active && !self.is_expired()
    }

    pub fn days_until_expiry(&self) -> i64 {
        if let Ok(exp) = chrono::DateTime::parse_from_rfc3339(&self.expires_at) {
            let diff = exp.signed_duration_since(chrono::Utc::now());
            diff.num_days()
        } else {
            0
        }
    }

    pub fn transfer(&mut self, new_owner: &str) {
        let now = chrono::Utc::now().to_rfc3339();
        let old = self.owner.clone();
        self.owner = new_owner.to_string();
        self.transfer_history
            .push((old, new_owner.to_string(), now));
    }

    pub fn renew(&mut self, new_expires_at: &str) {
        self.expires_at = new_expires_at.to_string();
    }
}

// ──────────────────────────── NameServiceStats ──────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NameServiceStats {
    pub total_names: usize,
    pub active: usize,
    pub expired: usize,
    pub reserved: usize,
    pub total_records: usize,
}

// ──────────────────────────── NameService ────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NameService {
    pub names: HashMap<String, RegisteredName>,
    pub reverse: HashMap<String, String>,
    pub reserved: Vec<String>,
}

impl NameService {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, name: RegisteredName) -> Result<(), NameServiceError> {
        if self.reserved.contains(&name.name) {
            return Err(NameServiceError::Reserved(name.name));
        }
        if self.names.contains_key(&name.name) {
            return Err(NameServiceError::AlreadyExists(name.name));
        }
        self.names.insert(name.name.clone(), name);
        Ok(())
    }

    pub fn unregister(&mut self, name: &str) -> Result<RegisteredName, NameServiceError> {
        let entry = self
            .names
            .remove(name)
            .ok_or_else(|| NameServiceError::NotFound(name.to_string()))?;
        // clean up reverse mapping if it pointed at this name
        self.reverse.retain(|_, v| v != name);
        Ok(entry)
    }

    pub fn get(&self, name: &str) -> Option<&RegisteredName> {
        self.names.get(name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut RegisteredName> {
        self.names.get_mut(name)
    }

    /// Resolve a name to its owner address (only if active).
    pub fn resolve(&self, name: &str) -> Option<&str> {
        self.names
            .get(name)
            .filter(|n| n.is_active())
            .map(|n| n.owner.as_str())
    }

    /// Reverse-resolve an address to its primary name.
    pub fn reverse_resolve(&self, address: &str) -> Option<&str> {
        self.reverse.get(address).map(|s| s.as_str())
    }

    /// Set the primary name for an address.
    pub fn set_primary(&mut self, address: &str, name: &str) -> Result<(), NameServiceError> {
        let entry = self
            .names
            .get(name)
            .ok_or_else(|| NameServiceError::NotFound(name.to_string()))?;
        if entry.owner != address {
            return Err(NameServiceError::NotOwner(format!(
                "{} does not own {}",
                address, name
            )));
        }
        self.reverse.insert(address.to_string(), name.to_string());
        Ok(())
    }

    pub fn transfer(&mut self, name: &str, new_owner: &str) -> Result<(), NameServiceError> {
        let entry = self
            .names
            .get_mut(name)
            .ok_or_else(|| NameServiceError::NotFound(name.to_string()))?;
        let old_owner = entry.owner.clone();
        entry.transfer(new_owner);
        // update reverse mapping: if old owner's primary was this name, remove it
        if self.reverse.get(&old_owner).map(|s| s.as_str()) == Some(name) {
            self.reverse.remove(&old_owner);
        }
        Ok(())
    }

    pub fn renew(&mut self, name: &str, new_expires_at: &str) -> Result<(), NameServiceError> {
        let entry = self
            .names
            .get_mut(name)
            .ok_or_else(|| NameServiceError::NotFound(name.to_string()))?;
        entry.renew(new_expires_at);
        Ok(())
    }

    pub fn set_record(&mut self, name: &str, record: NameRecord) -> Result<(), NameServiceError> {
        let entry = self
            .names
            .get_mut(name)
            .ok_or_else(|| NameServiceError::NotFound(name.to_string()))?;
        // replace if same key exists, otherwise add
        entry.remove_record(&record.key);
        entry.add_record(record);
        Ok(())
    }

    pub fn reserve_name(&mut self, name: &str) {
        if !self.reserved.contains(&name.to_string()) {
            self.reserved.push(name.to_string());
        }
    }

    pub fn unreserve_name(&mut self, name: &str) -> bool {
        let before = self.reserved.len();
        self.reserved.retain(|n| n != name);
        self.reserved.len() < before
    }

    pub fn is_available(&self, name: &str) -> bool {
        !self.names.contains_key(name) && !self.reserved.contains(&name.to_string())
    }

    /// Case-insensitive search on name.
    pub fn search(&self, query: &str) -> Vec<&RegisteredName> {
        let q = query.to_lowercase();
        self.names
            .values()
            .filter(|n| n.name.to_lowercase().contains(&q))
            .collect()
    }

    pub fn names_by_owner(&self, owner: &str) -> Vec<&RegisteredName> {
        self.names.values().filter(|n| n.owner == owner).collect()
    }

    /// Names expiring within the given number of days (and not already expired).
    pub fn expiring_soon(&self, days: i64) -> Vec<&RegisteredName> {
        self.names
            .values()
            .filter(|n| {
                let d = n.days_until_expiry();
                d >= 0 && d <= days
            })
            .collect()
    }

    pub fn expired_names(&self) -> Vec<&RegisteredName> {
        self.names.values().filter(|n| n.is_expired()).collect()
    }

    pub fn stats(&self) -> NameServiceStats {
        let total_names = self.names.len();
        let active = self.names.values().filter(|n| n.is_active()).count();
        let expired = self.names.values().filter(|n| n.is_expired()).count();
        let reserved = self.reserved.len();
        let total_records: usize = self.names.values().map(|n| n.records.len()).sum();
        NameServiceStats {
            total_names,
            active,
            expired,
            reserved,
            total_records,
        }
    }

    // ──────────────────── Persistence ────────────────────────────────────

    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), NameServiceError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, NameServiceError> {
        let data = std::fs::read_to_string(path)?;
        let svc: NameService = serde_json::from_str(&data)?;
        Ok(svc)
    }

    pub fn load_or_default<P: AsRef<Path>>(path: P) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ──────────────────────────── Tests ─────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const FUTURE: &str = "2099-01-01T00:00:00+00:00";
    const PAST: &str = "2020-01-01T00:00:00+00:00";

    fn alice() -> RegisteredName {
        RegisteredName::new("alice.evap", "0xAlice", FUTURE)
    }

    fn bob() -> RegisteredName {
        RegisteredName::new("bob.evap", "0xBob", FUTURE)
    }

    fn svc() -> NameService {
        NameService::new()
    }

    fn test_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("name_svc_test_{}", std::process::id()))
    }

    #[test]
    fn test_register_and_get() {
        let mut ns = svc();
        ns.register(alice()).unwrap();
        let got = ns.get("alice.evap").unwrap();
        assert_eq!(got.owner, "0xAlice");
        assert_eq!(got.status, NameStatus::Active);
    }

    #[test]
    fn test_register_duplicate_rejected() {
        let mut ns = svc();
        ns.register(alice()).unwrap();
        let err = ns.register(alice()).unwrap_err();
        assert!(matches!(err, NameServiceError::AlreadyExists(_)));
    }

    #[test]
    fn test_register_reserved_rejected() {
        let mut ns = svc();
        ns.reserve_name("admin.evap");
        let name = RegisteredName::new("admin.evap", "0xEvil", FUTURE);
        let err = ns.register(name).unwrap_err();
        assert!(matches!(err, NameServiceError::Reserved(_)));
    }

    #[test]
    fn test_unregister() {
        let mut ns = svc();
        ns.register(alice()).unwrap();
        let removed = ns.unregister("alice.evap").unwrap();
        assert_eq!(removed.name, "alice.evap");
        assert!(ns.get("alice.evap").is_none());
    }

    #[test]
    fn test_resolve_active() {
        let mut ns = svc();
        ns.register(alice()).unwrap();
        assert_eq!(ns.resolve("alice.evap"), Some("0xAlice"));
    }

    #[test]
    fn test_resolve_expired_returns_none() {
        let mut ns = svc();
        let expired = RegisteredName::new("old.evap", "0xOld", PAST);
        ns.register(expired).unwrap();
        assert_eq!(ns.resolve("old.evap"), None);
    }

    #[test]
    fn test_reverse_resolve() {
        let mut ns = svc();
        ns.register(alice()).unwrap();
        ns.set_primary("0xAlice", "alice.evap").unwrap();
        assert_eq!(ns.reverse_resolve("0xAlice"), Some("alice.evap"));
    }

    #[test]
    fn test_set_primary() {
        let mut ns = svc();
        ns.register(alice()).unwrap();
        ns.set_primary("0xAlice", "alice.evap").unwrap();
        assert_eq!(ns.reverse_resolve("0xAlice"), Some("alice.evap"));

        // wrong owner fails
        let err = ns.set_primary("0xBob", "alice.evap").unwrap_err();
        assert!(matches!(err, NameServiceError::NotOwner(_)));
    }

    #[test]
    fn test_transfer() {
        let mut ns = svc();
        ns.register(alice()).unwrap();
        ns.set_primary("0xAlice", "alice.evap").unwrap();
        ns.transfer("alice.evap", "0xBob").unwrap();

        let entry = ns.get("alice.evap").unwrap();
        assert_eq!(entry.owner, "0xBob");
        assert_eq!(entry.transfer_history.len(), 1);
        assert_eq!(entry.transfer_history[0].0, "0xAlice");
        assert_eq!(entry.transfer_history[0].1, "0xBob");
        // reverse mapping for old owner should be cleared
        assert!(ns.reverse_resolve("0xAlice").is_none());
    }

    #[test]
    fn test_renew() {
        let mut ns = svc();
        let expired = RegisteredName::new("renew.evap", "0xOwner", PAST);
        ns.register(expired).unwrap();
        assert!(ns.resolve("renew.evap").is_none());

        ns.renew("renew.evap", FUTURE).unwrap();
        assert_eq!(ns.resolve("renew.evap"), Some("0xOwner"));
    }

    #[test]
    fn test_set_record() {
        let mut ns = svc();
        ns.register(alice()).unwrap();
        let rec = NameRecord::new(RecordType::Email, "alice@evap.io".to_string());
        ns.set_record("alice.evap", rec).unwrap();
        let entry = ns.get("alice.evap").unwrap();
        assert_eq!(entry.get_record(&RecordType::Email), Some("alice@evap.io"));
    }

    #[test]
    fn test_get_record() {
        let mut name = alice();
        name.add_record(NameRecord::new(
            RecordType::Avatar,
            "https://img.png".to_string(),
        ));
        assert_eq!(
            name.get_record(&RecordType::Avatar),
            Some("https://img.png")
        );
        assert_eq!(name.get_record(&RecordType::Email), None);
    }

    #[test]
    fn test_reserve_and_unreserve() {
        let mut ns = svc();
        ns.reserve_name("protocol.evap");
        assert!(!ns.is_available("protocol.evap"));
        assert!(ns.unreserve_name("protocol.evap"));
        assert!(ns.is_available("protocol.evap"));
        // unreserve again returns false
        assert!(!ns.unreserve_name("protocol.evap"));
    }

    #[test]
    fn test_is_available() {
        let mut ns = svc();
        assert!(ns.is_available("free.evap"));
        ns.register(alice()).unwrap();
        assert!(!ns.is_available("alice.evap"));
        ns.reserve_name("vip.evap");
        assert!(!ns.is_available("vip.evap"));
    }

    #[test]
    fn test_search() {
        let mut ns = svc();
        ns.register(alice()).unwrap();
        ns.register(bob()).unwrap();
        let results = ns.search("ALICE");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "alice.evap");
    }

    #[test]
    fn test_names_by_owner() {
        let mut ns = svc();
        ns.register(alice()).unwrap();
        let a2 = RegisteredName::new("alice2.evap", "0xAlice", FUTURE);
        ns.register(a2).unwrap();
        ns.register(bob()).unwrap();
        let owned = ns.names_by_owner("0xAlice");
        assert_eq!(owned.len(), 2);
    }

    #[test]
    fn test_expiring_soon() {
        let mut ns = svc();
        // name expiring far in the future
        ns.register(alice()).unwrap();
        // name already expired
        let old = RegisteredName::new("old.evap", "0xOld", PAST);
        ns.register(old).unwrap();

        let soon = ns.expiring_soon(365);
        // alice expires in ~73 years — not "soon" at 365 days unless days_until_expiry <= 365
        // old is expired (negative), should not appear
        // so nothing should match for 365 days with our test data
        assert!(soon.iter().all(|n| n.days_until_expiry() >= 0));
        assert!(soon.iter().all(|n| n.days_until_expiry() <= 365));
    }

    #[test]
    fn test_expired_names() {
        let mut ns = svc();
        ns.register(alice()).unwrap();
        let old = RegisteredName::new("old.evap", "0xOld", PAST);
        ns.register(old).unwrap();
        let expired = ns.expired_names();
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].name, "old.evap");
    }

    #[test]
    fn test_subnames() {
        let mut name = alice();
        name.add_subname("wallet.alice.evap");
        name.add_subname("nft.alice.evap");
        assert_eq!(name.subnames.len(), 2);
        assert!(name.subnames.contains(&"wallet.alice.evap".to_string()));
        assert!(name.subnames.contains(&"nft.alice.evap".to_string()));
    }

    #[test]
    fn test_stats() {
        let mut ns = svc();
        ns.register(alice()).unwrap();
        let old = RegisteredName::new("old.evap", "0xOld", PAST);
        ns.register(old).unwrap();
        ns.reserve_name("vip.evap");
        ns.set_record(
            "alice.evap",
            NameRecord::new(RecordType::Email, "a@b.c".to_string()),
        )
        .unwrap();

        let s = ns.stats();
        assert_eq!(s.total_names, 2);
        assert_eq!(s.active, 1);
        assert_eq!(s.expired, 1);
        assert_eq!(s.reserved, 1);
        assert_eq!(s.total_records, 1);
    }

    #[test]
    fn test_persistence_roundtrip() {
        let path = test_path();
        let mut ns = svc();
        ns.register(alice()).unwrap();
        ns.set_primary("0xAlice", "alice.evap").unwrap();
        ns.reserve_name("vip.evap");
        ns.save(&path).unwrap();

        let loaded = NameService::load(&path).unwrap();
        assert_eq!(loaded.names.len(), 1);
        assert_eq!(loaded.reverse_resolve("0xAlice"), Some("alice.evap"));
        assert_eq!(loaded.reserved.len(), 1);

        // also test load_or_default
        let loaded2 = NameService::load_or_default(&path);
        assert_eq!(loaded2.names.len(), 1);

        // load_or_default on missing path gives empty
        let missing = std::env::temp_dir().join("name_svc_nonexistent_42");
        let empty = NameService::load_or_default(&missing);
        assert_eq!(empty.names.len(), 0);

        let _ = std::fs::remove_file(&path);
    }
}
