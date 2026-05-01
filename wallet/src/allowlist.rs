// wallet/src/allowlist.rs — Address allowlist/denylist management
//
// Maintain approved and blocked address lists with:
//   - Per-entry notes and expiry
//   - Prefix pattern matching (e.g. "evap1abc*")
//   - CSV/JSON import/export
//   - Check API for transaction gating

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AllowlistError {
    #[error("address already in list: {0}")]
    AlreadyExists(String),
    #[error("address not found: {0}")]
    NotFound(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("json error: {0}")]
    Json(String),
    #[error("csv parse error: {0}")]
    CsvParse(String),
}

// ── Entry types ───────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ListType {
    Allow,
    Deny,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListEntry {
    pub address: String,
    pub list_type: ListType,
    pub note: String,
    pub added_at: String,
    pub expires_at: Option<String>,
    pub added_by: String,
    pub tags: Vec<String>,
}

impl ListEntry {
    pub fn is_expired(&self, now_ts: &str) -> bool {
        match &self.expires_at {
            Some(exp) => exp.as_str() <= now_ts,
            None => false,
        }
    }

    pub fn is_pattern(&self) -> bool {
        self.address.contains('*')
    }

    /// Check if a given address matches this entry (exact or wildcard prefix)
    pub fn matches(&self, addr: &str) -> bool {
        if self.address.contains('*') {
            let prefix = self.address.trim_end_matches('*');
            addr.starts_with(prefix)
        } else {
            self.address == addr
        }
    }
}

// ── Verdict ───────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    Allowed,
    Denied { reason: String },
    NotListed,
}

impl Verdict {
    pub fn is_denied(&self) -> bool {
        matches!(self, Verdict::Denied { .. })
    }

    pub fn is_allowed(&self) -> bool {
        matches!(self, Verdict::Allowed)
    }
}

// ── Allowlist store ───────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DefaultPolicy {
    AllowAll,
    DenyAll,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddressListStore {
    pub entries: HashMap<String, ListEntry>,
    pub default_policy: DefaultPolicy,
}

impl Default for AddressListStore {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            default_policy: DefaultPolicy::AllowAll,
        }
    }
}

impl AddressListStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_default_deny() -> Self {
        Self {
            entries: HashMap::new(),
            default_policy: DefaultPolicy::DenyAll,
        }
    }

    /// Add an address to allow or deny list
    pub fn add(
        &mut self,
        address: &str,
        list_type: ListType,
        note: &str,
        added_by: &str,
    ) -> Result<(), AllowlistError> {
        if self.entries.contains_key(address) {
            return Err(AllowlistError::AlreadyExists(address.into()));
        }
        self.entries.insert(
            address.to_string(),
            ListEntry {
                address: address.to_string(),
                list_type,
                note: note.to_string(),
                added_at: chrono::Utc::now().to_rfc3339(),
                expires_at: None,
                added_by: added_by.to_string(),
                tags: Vec::new(),
            },
        );
        Ok(())
    }

    /// Add with expiry
    pub fn add_with_expiry(
        &mut self,
        address: &str,
        list_type: ListType,
        note: &str,
        added_by: &str,
        expires_at: &str,
    ) -> Result<(), AllowlistError> {
        if self.entries.contains_key(address) {
            return Err(AllowlistError::AlreadyExists(address.into()));
        }
        self.entries.insert(
            address.to_string(),
            ListEntry {
                address: address.to_string(),
                list_type,
                note: note.to_string(),
                added_at: chrono::Utc::now().to_rfc3339(),
                expires_at: Some(expires_at.to_string()),
                added_by: added_by.to_string(),
                tags: Vec::new(),
            },
        );
        Ok(())
    }

    /// Remove an address
    pub fn remove(&mut self, address: &str) -> Result<ListEntry, AllowlistError> {
        self.entries
            .remove(address)
            .ok_or_else(|| AllowlistError::NotFound(address.into()))
    }

    /// Check an address against the lists
    pub fn check(&self, address: &str) -> Verdict {
        let now = chrono::Utc::now().to_rfc3339();

        // Check exact match first, then patterns
        for entry in self.entries.values() {
            if entry.is_expired(&now) {
                continue;
            }
            if entry.matches(address) {
                return match entry.list_type {
                    ListType::Deny => Verdict::Denied {
                        reason: if entry.note.is_empty() {
                            "address is on deny list".to_string()
                        } else {
                            entry.note.clone()
                        },
                    },
                    ListType::Allow => Verdict::Allowed,
                };
            }
        }

        // No match — apply default policy
        match self.default_policy {
            DefaultPolicy::AllowAll => Verdict::NotListed,
            DefaultPolicy::DenyAll => Verdict::Denied {
                reason: "address not on allow list (default deny)".to_string(),
            },
        }
    }

    /// Get all allowed addresses
    pub fn allowed(&self) -> Vec<&ListEntry> {
        self.entries
            .values()
            .filter(|e| e.list_type == ListType::Allow)
            .collect()
    }

    /// Get all denied addresses
    pub fn denied(&self) -> Vec<&ListEntry> {
        self.entries
            .values()
            .filter(|e| e.list_type == ListType::Deny)
            .collect()
    }

    /// Get entries by tag
    pub fn by_tag(&self, tag: &str) -> Vec<&ListEntry> {
        self.entries
            .values()
            .filter(|e| e.tags.iter().any(|t| t == tag))
            .collect()
    }

    /// Remove all expired entries
    pub fn purge_expired(&mut self) -> usize {
        let now = chrono::Utc::now().to_rfc3339();
        let before = self.entries.len();
        self.entries.retain(|_, e| !e.is_expired(&now));
        before - self.entries.len()
    }

    /// Tag an entry
    pub fn tag(&mut self, address: &str, tag: &str) -> Result<(), AllowlistError> {
        let entry = self
            .entries
            .get_mut(address)
            .ok_or_else(|| AllowlistError::NotFound(address.into()))?;
        if !entry.tags.contains(&tag.to_string()) {
            entry.tags.push(tag.to_string());
        }
        Ok(())
    }

    /// Export to CSV
    pub fn to_csv(&self) -> String {
        let mut out = String::from("address,type,note,added_at,expires_at,added_by\n");
        for e in self.entries.values() {
            let list_type = match e.list_type {
                ListType::Allow => "allow",
                ListType::Deny => "deny",
            };
            out.push_str(&format!(
                "{},{},{},{},{},{}\n",
                e.address,
                list_type,
                e.note.replace(',', ";"),
                e.added_at,
                e.expires_at.as_deref().unwrap_or(""),
                e.added_by,
            ));
        }
        out
    }

    /// Import from CSV string
    pub fn import_csv(&mut self, csv: &str) -> Result<usize, AllowlistError> {
        let mut count = 0;
        for (i, line) in csv.lines().enumerate() {
            if i == 0 && line.contains("address") {
                continue; // Skip header
            }
            let parts: Vec<&str> = line.splitn(6, ',').collect();
            if parts.len() < 2 {
                continue;
            }
            let address = parts[0].trim();
            let list_type = match parts[1].trim() {
                "allow" => ListType::Allow,
                "deny" => ListType::Deny,
                other => {
                    return Err(AllowlistError::CsvParse(format!(
                        "line {}: unknown type '{}'",
                        i + 1,
                        other
                    )))
                }
            };
            let note = parts.get(2).unwrap_or(&"").trim();
            if self.entries.contains_key(address) {
                continue; // Skip duplicates
            }
            let added_by = parts.get(5).unwrap_or(&"csv-import").trim();
            self.entries.insert(
                address.to_string(),
                ListEntry {
                    address: address.to_string(),
                    list_type,
                    note: note.to_string(),
                    added_at: chrono::Utc::now().to_rfc3339(),
                    expires_at: None,
                    added_by: added_by.to_string(),
                    tags: vec!["imported".to_string()],
                },
            );
            count += 1;
        }
        Ok(count)
    }

    /// JSON persistence
    pub fn save(&self, path: &Path) -> Result<(), AllowlistError> {
        let json =
            serde_json::to_string_pretty(self).map_err(|e| AllowlistError::Json(e.to_string()))?;
        std::fs::write(path, json).map_err(|e| AllowlistError::Io(e.to_string()))
    }

    pub fn load(path: &Path) -> Result<Self, AllowlistError> {
        let data = std::fs::read_to_string(path).map_err(|e| AllowlistError::Io(e.to_string()))?;
        serde_json::from_str(&data).map_err(|e| AllowlistError::Json(e.to_string()))
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ── Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_allow() {
        let mut store = AddressListStore::new();
        store
            .add("evap1abc", ListType::Allow, "trusted", "me")
            .unwrap();
        assert_eq!(store.entries.len(), 1);
        assert_eq!(store.allowed().len(), 1);
    }

    #[test]
    fn test_add_deny() {
        let mut store = AddressListStore::new();
        store
            .add("evap1bad", ListType::Deny, "scammer", "me")
            .unwrap();
        assert_eq!(store.denied().len(), 1);
    }

    #[test]
    fn test_add_duplicate() {
        let mut store = AddressListStore::new();
        store.add("evap1abc", ListType::Allow, "", "me").unwrap();
        assert!(store.add("evap1abc", ListType::Allow, "", "me").is_err());
    }

    #[test]
    fn test_remove() {
        let mut store = AddressListStore::new();
        store.add("evap1abc", ListType::Allow, "", "me").unwrap();
        let entry = store.remove("evap1abc").unwrap();
        assert_eq!(entry.address, "evap1abc");
        assert!(store.entries.is_empty());
    }

    #[test]
    fn test_remove_not_found() {
        let mut store = AddressListStore::new();
        assert!(store.remove("nope").is_err());
    }

    #[test]
    fn test_check_allowed() {
        let mut store = AddressListStore::new();
        store.add("evap1abc", ListType::Allow, "", "me").unwrap();
        assert!(store.check("evap1abc").is_allowed());
    }

    #[test]
    fn test_check_denied() {
        let mut store = AddressListStore::new();
        store.add("evap1bad", ListType::Deny, "scam", "me").unwrap();
        let v = store.check("evap1bad");
        assert!(v.is_denied());
    }

    #[test]
    fn test_check_not_listed_allow_all() {
        let store = AddressListStore::new();
        assert_eq!(store.check("evap1xyz"), Verdict::NotListed);
    }

    #[test]
    fn test_check_not_listed_deny_all() {
        let store = AddressListStore::with_default_deny();
        assert!(store.check("evap1xyz").is_denied());
    }

    #[test]
    fn test_wildcard_pattern_allow() {
        let mut store = AddressListStore::new();
        store
            .add("evap1abc*", ListType::Allow, "prefix", "me")
            .unwrap();
        assert!(store.check("evap1abc123").is_allowed());
        assert!(store.check("evap1abcxyz").is_allowed());
        assert_eq!(store.check("evap1def"), Verdict::NotListed);
    }

    #[test]
    fn test_wildcard_pattern_deny() {
        let mut store = AddressListStore::new();
        store
            .add("evap1bad*", ListType::Deny, "bad prefix", "me")
            .unwrap();
        assert!(store.check("evap1bad999").is_denied());
    }

    #[test]
    fn test_is_pattern() {
        let entry = ListEntry {
            address: "evap1abc*".to_string(),
            list_type: ListType::Allow,
            note: String::new(),
            added_at: String::new(),
            expires_at: None,
            added_by: String::new(),
            tags: Vec::new(),
        };
        assert!(entry.is_pattern());
    }

    #[test]
    fn test_expiry() {
        let mut store = AddressListStore::new();
        store
            .add_with_expiry(
                "evap1exp",
                ListType::Deny,
                "temp",
                "me",
                "2020-01-01T00:00:00Z",
            )
            .unwrap();
        // Should be expired, so check should fall through to default
        assert_eq!(store.check("evap1exp"), Verdict::NotListed);
    }

    #[test]
    fn test_non_expired() {
        let mut store = AddressListStore::new();
        store
            .add_with_expiry(
                "evap1ok",
                ListType::Deny,
                "future",
                "me",
                "2099-01-01T00:00:00Z",
            )
            .unwrap();
        assert!(store.check("evap1ok").is_denied());
    }

    #[test]
    fn test_purge_expired() {
        let mut store = AddressListStore::new();
        store
            .add_with_expiry("evap1old", ListType::Deny, "", "me", "2020-01-01T00:00:00Z")
            .unwrap();
        store.add("evap1keep", ListType::Allow, "", "me").unwrap();
        let purged = store.purge_expired();
        assert_eq!(purged, 1);
        assert_eq!(store.entries.len(), 1);
    }

    #[test]
    fn test_tag() {
        let mut store = AddressListStore::new();
        store.add("evap1abc", ListType::Allow, "", "me").unwrap();
        store.tag("evap1abc", "exchange").unwrap();
        store.tag("evap1abc", "trusted").unwrap();
        // Duplicate tag should be idempotent
        store.tag("evap1abc", "trusted").unwrap();
        assert_eq!(store.entries["evap1abc"].tags.len(), 2);
    }

    #[test]
    fn test_by_tag() {
        let mut store = AddressListStore::new();
        store.add("evap1a", ListType::Allow, "", "me").unwrap();
        store.add("evap1b", ListType::Allow, "", "me").unwrap();
        store.tag("evap1a", "exchange").unwrap();
        assert_eq!(store.by_tag("exchange").len(), 1);
        assert_eq!(store.by_tag("other").len(), 0);
    }

    #[test]
    fn test_to_csv() {
        let mut store = AddressListStore::new();
        store
            .add("evap1abc", ListType::Allow, "friend", "me")
            .unwrap();
        store.add("evap1bad", ListType::Deny, "scam", "me").unwrap();
        let csv = store.to_csv();
        assert!(csv.contains("evap1abc,allow,friend"));
        assert!(csv.contains("evap1bad,deny,scam"));
    }

    #[test]
    fn test_import_csv() {
        let mut store = AddressListStore::new();
        let csv = "address,type,note,added_at,expires_at,added_by\nevap1x,allow,good,,, admin\nevap1y,deny,bad,,,admin\n";
        let count = store.import_csv(csv).unwrap();
        assert_eq!(count, 2);
        assert!(store.check("evap1x").is_allowed());
        assert!(store.check("evap1y").is_denied());
    }

    #[test]
    fn test_import_csv_skip_duplicates() {
        let mut store = AddressListStore::new();
        store.add("evap1x", ListType::Allow, "", "me").unwrap();
        let csv = "address,type\nevap1x,allow\nevap1y,deny\n";
        let count = store.import_csv(csv).unwrap();
        assert_eq!(count, 1); // Only evap1y imported
    }

    #[test]
    fn test_import_csv_bad_type() {
        let mut store = AddressListStore::new();
        let csv = "address,type\nevap1x,maybe\n";
        assert!(store.import_csv(csv).is_err());
    }

    #[test]
    fn test_verdict_methods() {
        assert!(Verdict::Allowed.is_allowed());
        assert!(!Verdict::Allowed.is_denied());
        assert!(Verdict::Denied { reason: "x".into() }.is_denied());
        assert!(!Verdict::NotListed.is_allowed());
        assert!(!Verdict::NotListed.is_denied());
    }

    #[test]
    fn test_save_load() {
        let path = std::env::temp_dir().join(format!("evap_allowlist_{}.json", std::process::id()));
        let mut store = AddressListStore::new();
        store
            .add("evap1abc", ListType::Allow, "test", "me")
            .unwrap();
        store.save(&path).unwrap();
        let loaded = AddressListStore::load(&path).unwrap();
        assert_eq!(loaded.entries.len(), 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_or_default() {
        let path = std::env::temp_dir().join("evap_allowlist_noexist.json");
        let store = AddressListStore::load_or_default(&path);
        assert!(store.entries.is_empty());
    }

    #[test]
    fn test_default_policy() {
        let store = AddressListStore::new();
        assert_eq!(store.default_policy, DefaultPolicy::AllowAll);
        let store2 = AddressListStore::with_default_deny();
        assert_eq!(store2.default_policy, DefaultPolicy::DenyAll);
    }
}
