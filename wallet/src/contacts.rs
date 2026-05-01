//! Address book — named contacts for easy transaction targeting.
//!
//! Stores human-readable labels for addresses, persisted to JSON.
//! Supports add, remove, lookup by name or address, and file I/O.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum ContactError {
    #[error("contact already exists: {0}")]
    Duplicate(String),
    #[error("contact not found: {0}")]
    NotFound(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

// ──────────────────────────── Contact ─────────────────────────────────

/// A single address book entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contact {
    /// Human-readable name (unique).
    pub name: String,
    /// Hex-encoded address (with 0x prefix).
    pub address: String,
    /// Optional note (e.g., "Alice's staking wallet").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Creation timestamp (ISO 8601).
    pub created_at: String,
}

// ──────────────────────────── AddressBook ─────────────────────────────

/// Persistent address book for named contacts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddressBook {
    pub version: u32,
    pub contacts: Vec<Contact>,
    /// Name → index for fast lookup.
    #[serde(skip)]
    name_index: HashMap<String, usize>,
}

impl AddressBook {
    /// Create a new empty address book.
    pub fn new() -> Self {
        Self {
            version: 1,
            contacts: Vec::new(),
            name_index: HashMap::new(),
        }
    }

    /// Rebuild the internal name index from contacts.
    fn rebuild_index(&mut self) {
        self.name_index.clear();
        for (i, c) in self.contacts.iter().enumerate() {
            self.name_index.insert(c.name.clone(), i);
        }
    }

    /// Load from a JSON file.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, ContactError> {
        let data = std::fs::read_to_string(path)?;
        let mut book: AddressBook = serde_json::from_str(&data)?;
        book.rebuild_index();
        Ok(book)
    }

    /// Save to a JSON file.
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), ContactError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Add a new contact. Returns error if name already exists.
    pub fn add(
        &mut self,
        name: &str,
        address: &str,
        note: Option<&str>,
    ) -> Result<(), ContactError> {
        if self.name_index.contains_key(name) {
            return Err(ContactError::Duplicate(name.to_string()));
        }

        let contact = Contact {
            name: name.to_string(),
            address: address.to_string(),
            note: note.map(|s| s.to_string()),
            created_at: chrono::Utc::now().to_rfc3339(),
        };

        let idx = self.contacts.len();
        self.contacts.push(contact);
        self.name_index.insert(name.to_string(), idx);
        Ok(())
    }

    /// Remove a contact by name.
    pub fn remove(&mut self, name: &str) -> Result<(), ContactError> {
        if !self.name_index.contains_key(name) {
            return Err(ContactError::NotFound(name.to_string()));
        }
        self.contacts.retain(|c| c.name != name);
        self.rebuild_index();
        Ok(())
    }

    /// Look up a contact by name.
    pub fn get_by_name(&self, name: &str) -> Option<&Contact> {
        self.name_index.get(name).map(|&i| &self.contacts[i])
    }

    /// Look up contacts by address (may return multiple).
    pub fn get_by_address(&self, address: &str) -> Vec<&Contact> {
        self.contacts
            .iter()
            .filter(|c| c.address == address)
            .collect()
    }

    /// Resolve a name-or-address string: if it matches a contact name,
    /// return the contact's address; otherwise return the input as-is.
    pub fn resolve(&self, name_or_address: &str) -> String {
        if let Some(contact) = self.get_by_name(name_or_address) {
            contact.address.clone()
        } else {
            name_or_address.to_string()
        }
    }

    /// List all contacts.
    pub fn list(&self) -> &[Contact] {
        &self.contacts
    }

    /// Number of contacts.
    pub fn len(&self) -> usize {
        self.contacts.len()
    }

    /// Whether the address book is empty.
    pub fn is_empty(&self) -> bool {
        self.contacts.is_empty()
    }

    /// Update a contact's address or note.
    pub fn update(
        &mut self,
        name: &str,
        new_address: Option<&str>,
        new_note: Option<Option<&str>>,
    ) -> Result<(), ContactError> {
        let idx = *self
            .name_index
            .get(name)
            .ok_or_else(|| ContactError::NotFound(name.to_string()))?;
        if let Some(addr) = new_address {
            self.contacts[idx].address = addr.to_string();
        }
        if let Some(note) = new_note {
            self.contacts[idx].note = note.map(|s| s.to_string());
        }
        Ok(())
    }

    /// Export contacts as CSV string.
    pub fn to_csv(&self) -> String {
        let mut csv = String::from("name,address,note\n");
        for c in &self.contacts {
            let note = c.note.as_deref().unwrap_or("");
            // Escape commas in note field
            let note_escaped = if note.contains(',') || note.contains('"') {
                format!("\"{}\"", note.replace('"', "\"\""))
            } else {
                note.to_string()
            };
            csv.push_str(&format!("{},{},{}\n", c.name, c.address, note_escaped));
        }
        csv
    }

    /// Export contacts to a CSV file.
    pub fn export_csv<P: AsRef<std::path::Path>>(&self, path: P) -> Result<(), ContactError> {
        std::fs::write(path, self.to_csv())?;
        Ok(())
    }

    /// Import contacts from a CSV string (name,address,note).
    /// Skips duplicates. Returns number of contacts imported.
    pub fn import_csv(&mut self, csv: &str) -> Result<usize, ContactError> {
        let mut imported = 0;
        for (i, line) in csv.lines().enumerate() {
            // Skip header
            if i == 0 && line.starts_with("name") {
                continue;
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let parts: Vec<&str> = trimmed.splitn(3, ',').collect();
            if parts.len() < 2 {
                continue;
            }

            let name = parts[0].trim();
            let address = parts[1].trim();
            let note = parts.get(2).map(|s| s.trim().trim_matches('"'));

            if name.is_empty() || address.is_empty() {
                continue;
            }

            // Skip if name already exists
            if self.name_index.contains_key(name) {
                continue;
            }

            self.add(name, address, note)?;
            imported += 1;
        }
        Ok(imported)
    }

    /// Import contacts from a CSV file.
    pub fn import_csv_file<P: AsRef<std::path::Path>>(
        &mut self,
        path: P,
    ) -> Result<usize, ContactError> {
        let data = std::fs::read_to_string(path)?;
        self.import_csv(&data)
    }

    /// Export contacts as JSON string.
    pub fn export_json(&self) -> Result<String, ContactError> {
        serde_json::to_string_pretty(&self.contacts).map_err(ContactError::from)
    }

    /// Import contacts from a JSON string.
    pub fn import_json(&mut self, json: &str) -> Result<usize, ContactError> {
        let contacts: Vec<Contact> = serde_json::from_str(json)?;
        let mut imported = 0;
        for c in contacts {
            if self.name_index.contains_key(&c.name) {
                continue;
            }
            self.add(&c.name, &c.address, c.note.as_deref())?;
            imported += 1;
        }
        Ok(imported)
    }
}

impl Default for AddressBook {
    fn default() -> Self {
        Self::new()
    }
}

// ──────────────────────────── Tests ──────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_and_lookup() {
        let mut book = AddressBook::new();
        book.add("alice", "0xabc123", Some("main wallet")).unwrap();
        let c = book.get_by_name("alice").unwrap();
        assert_eq!(c.address, "0xabc123");
        assert_eq!(c.note.as_deref(), Some("main wallet"));
    }

    #[test]
    fn test_duplicate_name_rejected() {
        let mut book = AddressBook::new();
        book.add("bob", "0x111", None).unwrap();
        let err = book.add("bob", "0x222", None);
        assert!(matches!(err, Err(ContactError::Duplicate(_))));
    }

    #[test]
    fn test_remove() {
        let mut book = AddressBook::new();
        book.add("temp", "0x999", None).unwrap();
        assert_eq!(book.len(), 1);
        book.remove("temp").unwrap();
        assert_eq!(book.len(), 0);
        assert!(book.get_by_name("temp").is_none());
    }

    #[test]
    fn test_remove_not_found() {
        let mut book = AddressBook::new();
        let err = book.remove("nobody");
        assert!(matches!(err, Err(ContactError::NotFound(_))));
    }

    #[test]
    fn test_get_by_address() {
        let mut book = AddressBook::new();
        book.add("alice", "0xaaa", None).unwrap();
        book.add("bob", "0xbbb", None).unwrap();
        book.add("alice-staking", "0xaaa", None).unwrap();

        let matches = book.get_by_address("0xaaa");
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn test_resolve_name() {
        let mut book = AddressBook::new();
        book.add("alice", "0xabc", None).unwrap();
        assert_eq!(book.resolve("alice"), "0xabc");
    }

    #[test]
    fn test_resolve_raw_address() {
        let book = AddressBook::new();
        assert_eq!(book.resolve("0xdeadbeef"), "0xdeadbeef");
    }

    #[test]
    fn test_update_contact() {
        let mut book = AddressBook::new();
        book.add("alice", "0xold", None).unwrap();
        book.update("alice", Some("0xnew"), Some(Some("updated")))
            .unwrap();
        let c = book.get_by_name("alice").unwrap();
        assert_eq!(c.address, "0xnew");
        assert_eq!(c.note.as_deref(), Some("updated"));
    }

    #[test]
    fn test_json_roundtrip() {
        let mut book = AddressBook::new();
        book.add("alice", "0xabc", Some("friend")).unwrap();
        book.add("bob", "0xdef", None).unwrap();

        let json = serde_json::to_string_pretty(&book).unwrap();
        let mut loaded: AddressBook = serde_json::from_str(&json).unwrap();
        loaded.rebuild_index();

        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded.get_by_name("alice").unwrap().address, "0xabc");
    }

    #[test]
    fn test_file_save_and_load() {
        let dir = std::env::temp_dir().join("evaporchain_contacts_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("contacts.json");

        let mut book = AddressBook::new();
        book.add("charlie", "0x123", Some("test")).unwrap();
        book.save(&path).unwrap();

        let loaded = AddressBook::load(&path).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded.get_by_name("charlie").unwrap().address, "0x123");

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_csv_export() {
        let mut book = AddressBook::new();
        book.add("alice", "0xabc", Some("friend")).unwrap();
        book.add("bob", "0xdef", None).unwrap();

        let csv = book.to_csv();
        assert!(csv.starts_with("name,address,note\n"));
        assert!(csv.contains("alice,0xabc,friend"));
        assert!(csv.contains("bob,0xdef,"));
    }

    #[test]
    fn test_csv_import() {
        let mut book = AddressBook::new();
        let csv = "name,address,note\nalice,0xabc,friend\nbob,0xdef,\n";
        let count = book.import_csv(csv).unwrap();
        assert_eq!(count, 2);
        assert_eq!(book.len(), 2);
        assert_eq!(book.get_by_name("alice").unwrap().address, "0xabc");
    }

    #[test]
    fn test_csv_import_skips_duplicates() {
        let mut book = AddressBook::new();
        book.add("alice", "0xold", None).unwrap();

        let csv = "name,address,note\nalice,0xnew,duplicate\nbob,0xdef,\n";
        let count = book.import_csv(csv).unwrap();
        assert_eq!(count, 1); // only bob imported
        assert_eq!(book.get_by_name("alice").unwrap().address, "0xold"); // unchanged
    }

    #[test]
    fn test_csv_import_skips_header() {
        let mut book = AddressBook::new();
        let csv = "name,address,note\ncharlie,0x999,\n";
        let count = book.import_csv(csv).unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_csv_import_skips_empty_lines() {
        let mut book = AddressBook::new();
        let csv = "name,address,note\n\nalice,0xabc,\n\n";
        let count = book.import_csv(csv).unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_json_export_import() {
        let mut book = AddressBook::new();
        book.add("alice", "0xabc", Some("friend")).unwrap();
        book.add("bob", "0xdef", None).unwrap();

        let json = book.export_json().unwrap();

        let mut book2 = AddressBook::new();
        let count = book2.import_json(&json).unwrap();
        assert_eq!(count, 2);
        assert_eq!(book2.get_by_name("alice").unwrap().address, "0xabc");
    }

    #[test]
    fn test_csv_file_roundtrip() {
        let mut book = AddressBook::new();
        book.add("alice", "0xabc", Some("test")).unwrap();

        let dir = std::env::temp_dir().join("evaporchain_contacts_csv_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("contacts.csv");

        book.export_csv(&path).unwrap();

        let mut book2 = AddressBook::new();
        let count = book2.import_csv_file(&path).unwrap();
        assert_eq!(count, 1);
        assert_eq!(
            book2.get_by_name("alice").unwrap().note.as_deref(),
            Some("test")
        );

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }
}
