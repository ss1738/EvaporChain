//! Auto-generated changelog module for the EvaporChain wallet.
//!
//! Provides structured changelog management with versioning, tagging,
//! searching, and persistence via serde_json.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Errors that can occur during changelog operations.
#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
pub enum ChangelogError {
    #[error("entry not found: {0}")]
    EntryNotFound(String),

    #[error("duplicate version: {0}")]
    DuplicateVersion(String),

    #[error("version not found: {0}")]
    VersionNotFound(String),

    #[error("io error: {0}")]
    Io(String),

    #[error("parse error: {0}")]
    Parse(String),
}

impl From<std::io::Error> for ChangelogError {
    fn from(e: std::io::Error) -> Self {
        ChangelogError::Io(e.to_string())
    }
}

impl From<serde_json::Error> for ChangelogError {
    fn from(e: serde_json::Error) -> Self {
        ChangelogError::Parse(e.to_string())
    }
}

/// The type of change recorded in an entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ChangeType {
    Added,
    Changed,
    Fixed,
    Removed,
    Security,
    Deprecated,
    Performance,
}

impl std::fmt::Display for ChangeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ChangeType::Added => "Added",
            ChangeType::Changed => "Changed",
            ChangeType::Fixed => "Fixed",
            ChangeType::Removed => "Removed",
            ChangeType::Security => "Security",
            ChangeType::Deprecated => "Deprecated",
            ChangeType::Performance => "Performance",
        };
        write!(f, "{}", s)
    }
}

/// The scope/area affected by a change.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ChangeScope {
    Wallet,
    Transaction,
    Energy,
    Staking,
    DeFi,
    Security,
    UI,
    Internal,
}

impl std::fmt::Display for ChangeScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ChangeScope::Wallet => "Wallet",
            ChangeScope::Transaction => "Transaction",
            ChangeScope::Energy => "Energy",
            ChangeScope::Staking => "Staking",
            ChangeScope::DeFi => "DeFi",
            ChangeScope::Security => "Security",
            ChangeScope::UI => "UI",
            ChangeScope::Internal => "Internal",
        };
        write!(f, "{}", s)
    }
}

/// A single changelog entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeEntry {
    pub id: String,
    pub change_type: ChangeType,
    pub scope: ChangeScope,
    pub description: String,
    pub author: String,
    pub timestamp: String,
    pub version: Option<String>,
    pub breaking: bool,
}

/// A version tag grouping a set of changelog entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionTag {
    pub version: String,
    pub name: String,
    pub entries: Vec<String>,
    pub tagged_at: String,
    pub notes: Option<String>,
}

/// Aggregate statistics about the changelog.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangelogStats {
    pub total_entries: usize,
    pub total_versions: usize,
    pub by_type: HashMap<String, usize>,
    pub by_scope: HashMap<String, usize>,
    pub breaking_changes: usize,
    pub latest_version: Option<String>,
}

/// The main changelog structure holding all entries and version tags.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Changelog {
    pub entries: HashMap<String, ChangeEntry>,
    pub versions: Vec<VersionTag>,
}

impl Changelog {
    /// Create a new empty changelog.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an entry to the changelog. Uses entry.id as the key.
    pub fn add_entry(&mut self, entry: ChangeEntry) -> Result<(), ChangelogError> {
        self.entries.insert(entry.id.clone(), entry);
        Ok(())
    }

    /// Remove an entry by id, returning it if found.
    pub fn remove_entry(&mut self, id: &str) -> Result<ChangeEntry, ChangelogError> {
        self.entries
            .remove(id)
            .ok_or_else(|| ChangelogError::EntryNotFound(id.to_string()))
    }

    /// Look up an entry by id.
    pub fn get_entry(&self, id: &str) -> Option<&ChangeEntry> {
        self.entries.get(id)
    }

    /// Tag a set of entry ids as a named version. Fails if the version already exists.
    pub fn tag_version(
        &mut self,
        version: &str,
        name: &str,
        entry_ids: Vec<String>,
        notes: Option<String>,
    ) -> Result<(), ChangelogError> {
        if self.versions.iter().any(|v| v.version == version) {
            return Err(ChangelogError::DuplicateVersion(version.to_string()));
        }

        // Set version on each entry that exists
        for eid in &entry_ids {
            if let Some(entry) = self.entries.get_mut(eid) {
                entry.version = Some(version.to_string());
            }
        }

        self.versions.push(VersionTag {
            version: version.to_string(),
            name: name.to_string(),
            entries: entry_ids,
            tagged_at: Utc::now().to_rfc3339(),
            notes,
        });

        Ok(())
    }

    /// Look up a version tag by version string.
    pub fn get_version(&self, version: &str) -> Option<&VersionTag> {
        self.versions.iter().find(|v| v.version == version)
    }

    /// Return all entries belonging to a specific version tag.
    pub fn entries_for_version(
        &self,
        version: &str,
    ) -> Result<Vec<&ChangeEntry>, ChangelogError> {
        let tag = self
            .versions
            .iter()
            .find(|v| v.version == version)
            .ok_or_else(|| ChangelogError::VersionNotFound(version.to_string()))?;

        let results: Vec<&ChangeEntry> = tag
            .entries
            .iter()
            .filter_map(|eid| self.entries.get(eid))
            .collect();

        Ok(results)
    }

    /// Return entries that have no version assigned.
    pub fn unversioned_entries(&self) -> Vec<&ChangeEntry> {
        self.entries
            .values()
            .filter(|e| e.version.is_none())
            .collect()
    }

    /// Return entries matching a specific change type.
    pub fn entries_by_type(&self, ct: &ChangeType) -> Vec<&ChangeEntry> {
        self.entries
            .values()
            .filter(|e| &e.change_type == ct)
            .collect()
    }

    /// Return entries matching a specific scope.
    pub fn entries_by_scope(&self, scope: &ChangeScope) -> Vec<&ChangeEntry> {
        self.entries
            .values()
            .filter(|e| &e.scope == scope)
            .collect()
    }

    /// Return all breaking-change entries.
    pub fn breaking_changes(&self) -> Vec<&ChangeEntry> {
        self.entries.values().filter(|e| e.breaking).collect()
    }

    /// Search entries by description or author (case-insensitive substring).
    pub fn search(&self, query: &str) -> Vec<&ChangeEntry> {
        let q = query.to_lowercase();
        self.entries
            .values()
            .filter(|e| {
                e.description.to_lowercase().contains(&q)
                    || e.author.to_lowercase().contains(&q)
            })
            .collect()
    }

    /// Return the most recent `n` entries, sorted by timestamp descending.
    pub fn recent_entries(&self, n: usize) -> Vec<&ChangeEntry> {
        let mut sorted: Vec<&ChangeEntry> = self.entries.values().collect();
        sorted.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        sorted.truncate(n);
        sorted
    }

    /// Render the changelog as a markdown string grouped by version.
    pub fn generate_markdown(&self) -> String {
        let mut md = String::from("# Changelog\n\n");

        // Versioned entries grouped by version tag
        for tag in self.versions.iter().rev() {
            md.push_str(&format!("## {} - {}\n\n", tag.version, tag.name));
            if let Some(ref notes) = tag.notes {
                md.push_str(&format!("{}\n\n", notes));
            }
            for eid in &tag.entries {
                if let Some(entry) = self.entries.get(eid) {
                    let breaking_marker = if entry.breaking { " **BREAKING**" } else { "" };
                    md.push_str(&format!(
                        "- [{}][{}] {}{} (by {})\n",
                        entry.change_type, entry.scope, entry.description, breaking_marker, entry.author
                    ));
                }
            }
            md.push('\n');
        }

        // Unversioned entries
        let unversioned = self.unversioned_entries();
        if !unversioned.is_empty() {
            md.push_str("## Unreleased\n\n");
            for entry in &unversioned {
                let breaking_marker = if entry.breaking { " **BREAKING**" } else { "" };
                md.push_str(&format!(
                    "- [{}][{}] {}{} (by {})\n",
                    entry.change_type, entry.scope, entry.description, breaking_marker, entry.author
                ));
            }
            md.push('\n');
        }

        md
    }

    /// Compute aggregate statistics for the changelog.
    pub fn stats(&self) -> ChangelogStats {
        let mut by_type: HashMap<String, usize> = HashMap::new();
        let mut by_scope: HashMap<String, usize> = HashMap::new();
        let mut breaking_changes = 0;

        for entry in self.entries.values() {
            *by_type
                .entry(entry.change_type.to_string())
                .or_insert(0) += 1;
            *by_scope.entry(entry.scope.to_string()).or_insert(0) += 1;
            if entry.breaking {
                breaking_changes += 1;
            }
        }

        let latest_version = self.versions.last().map(|v| v.version.clone());

        ChangelogStats {
            total_entries: self.entries.len(),
            total_versions: self.versions.len(),
            by_type,
            by_scope,
            breaking_changes,
            latest_version,
        }
    }

    /// Save the changelog to a JSON file.
    pub fn save(&self, path: &Path) -> Result<(), ChangelogError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Load a changelog from a JSON file.
    pub fn load(path: &Path) -> Result<Self, ChangelogError> {
        let data = std::fs::read_to_string(path)?;
        let changelog: Self = serde_json::from_str(&data)?;
        Ok(changelog)
    }

    /// Load a changelog from a JSON file, returning a default if the file
    /// does not exist or cannot be parsed.
    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(id: &str, ct: ChangeType, scope: ChangeScope, breaking: bool) -> ChangeEntry {
        ChangeEntry {
            id: id.to_string(),
            change_type: ct,
            scope,
            description: format!("Description for {}", id),
            author: "dev".to_string(),
            timestamp: Utc::now().to_rfc3339(),
            version: None,
            breaking,
        }
    }

    fn test_file_path(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir();
        dir.join(format!(
            "evaporchain_changelog_test_{}_{}_{}.json",
            name,
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ))
    }

    #[test]
    fn test_add_entry() {
        let mut cl = Changelog::new();
        let entry = make_entry("e1", ChangeType::Added, ChangeScope::Wallet, false);
        cl.add_entry(entry).unwrap();
        assert_eq!(cl.entries.len(), 1);
        assert!(cl.get_entry("e1").is_some());
    }

    #[test]
    fn test_remove_entry() {
        let mut cl = Changelog::new();
        cl.add_entry(make_entry("e1", ChangeType::Fixed, ChangeScope::Transaction, false))
            .unwrap();
        let removed = cl.remove_entry("e1").unwrap();
        assert_eq!(removed.id, "e1");
        assert!(cl.entries.is_empty());
    }

    #[test]
    fn test_remove_entry_not_found() {
        let mut cl = Changelog::new();
        let err = cl.remove_entry("nonexistent").unwrap_err();
        match err {
            ChangelogError::EntryNotFound(id) => assert_eq!(id, "nonexistent"),
            _ => panic!("expected EntryNotFound"),
        }
    }

    #[test]
    fn test_tag_version() {
        let mut cl = Changelog::new();
        cl.add_entry(make_entry("e1", ChangeType::Added, ChangeScope::Wallet, false))
            .unwrap();
        cl.add_entry(make_entry("e2", ChangeType::Fixed, ChangeScope::Energy, false))
            .unwrap();
        cl.tag_version(
            "1.0.0",
            "Initial Release",
            vec!["e1".into(), "e2".into()],
            Some("First release".into()),
        )
        .unwrap();

        assert_eq!(cl.versions.len(), 1);
        let tag = cl.get_version("1.0.0").unwrap();
        assert_eq!(tag.name, "Initial Release");
        assert_eq!(tag.entries.len(), 2);
        assert_eq!(tag.notes, Some("First release".into()));
    }

    #[test]
    fn test_duplicate_version() {
        let mut cl = Changelog::new();
        cl.tag_version("1.0.0", "v1", vec![], None).unwrap();
        let err = cl.tag_version("1.0.0", "v1 dup", vec![], None).unwrap_err();
        match err {
            ChangelogError::DuplicateVersion(v) => assert_eq!(v, "1.0.0"),
            _ => panic!("expected DuplicateVersion"),
        }
    }

    #[test]
    fn test_entries_for_version() {
        let mut cl = Changelog::new();
        cl.add_entry(make_entry("e1", ChangeType::Added, ChangeScope::Wallet, false))
            .unwrap();
        cl.tag_version("1.0.0", "v1", vec!["e1".into()], None)
            .unwrap();

        let entries = cl.entries_for_version("1.0.0").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "e1");
    }

    #[test]
    fn test_entries_for_version_not_found() {
        let cl = Changelog::new();
        let err = cl.entries_for_version("9.9.9").unwrap_err();
        match err {
            ChangelogError::VersionNotFound(v) => assert_eq!(v, "9.9.9"),
            _ => panic!("expected VersionNotFound"),
        }
    }

    #[test]
    fn test_unversioned_entries() {
        let mut cl = Changelog::new();
        cl.add_entry(make_entry("e1", ChangeType::Added, ChangeScope::Wallet, false))
            .unwrap();
        cl.add_entry(make_entry("e2", ChangeType::Changed, ChangeScope::UI, false))
            .unwrap();
        cl.tag_version("1.0.0", "v1", vec!["e1".into()], None)
            .unwrap();

        let unversioned = cl.unversioned_entries();
        assert_eq!(unversioned.len(), 1);
        assert_eq!(unversioned[0].id, "e2");
    }

    #[test]
    fn test_entries_by_type() {
        let mut cl = Changelog::new();
        cl.add_entry(make_entry("e1", ChangeType::Added, ChangeScope::Wallet, false))
            .unwrap();
        cl.add_entry(make_entry("e2", ChangeType::Fixed, ChangeScope::Energy, false))
            .unwrap();
        cl.add_entry(make_entry("e3", ChangeType::Added, ChangeScope::UI, false))
            .unwrap();

        let added = cl.entries_by_type(&ChangeType::Added);
        assert_eq!(added.len(), 2);
    }

    #[test]
    fn test_entries_by_scope() {
        let mut cl = Changelog::new();
        cl.add_entry(make_entry("e1", ChangeType::Added, ChangeScope::Wallet, false))
            .unwrap();
        cl.add_entry(make_entry("e2", ChangeType::Fixed, ChangeScope::Wallet, false))
            .unwrap();
        cl.add_entry(make_entry("e3", ChangeType::Changed, ChangeScope::Energy, false))
            .unwrap();

        let wallet = cl.entries_by_scope(&ChangeScope::Wallet);
        assert_eq!(wallet.len(), 2);
    }

    #[test]
    fn test_breaking_changes() {
        let mut cl = Changelog::new();
        cl.add_entry(make_entry("e1", ChangeType::Changed, ChangeScope::Wallet, true))
            .unwrap();
        cl.add_entry(make_entry("e2", ChangeType::Added, ChangeScope::UI, false))
            .unwrap();
        cl.add_entry(make_entry("e3", ChangeType::Removed, ChangeScope::DeFi, true))
            .unwrap();

        let breaking = cl.breaking_changes();
        assert_eq!(breaking.len(), 2);
    }

    #[test]
    fn test_search_description() {
        let mut cl = Changelog::new();
        let mut entry = make_entry("e1", ChangeType::Added, ChangeScope::Wallet, false);
        entry.description = "Add wallet backup feature".to_string();
        cl.add_entry(entry).unwrap();

        let mut entry2 = make_entry("e2", ChangeType::Fixed, ChangeScope::Energy, false);
        entry2.description = "Fix energy calculation".to_string();
        cl.add_entry(entry2).unwrap();

        let results = cl.search("backup");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "e1");
    }

    #[test]
    fn test_search_author() {
        let mut cl = Changelog::new();
        let mut entry = make_entry("e1", ChangeType::Added, ChangeScope::Wallet, false);
        entry.author = "Alice".to_string();
        cl.add_entry(entry).unwrap();

        let mut entry2 = make_entry("e2", ChangeType::Fixed, ChangeScope::Energy, false);
        entry2.author = "Bob".to_string();
        cl.add_entry(entry2).unwrap();

        let results = cl.search("alice");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "e1");
    }

    #[test]
    fn test_recent_entries() {
        let mut cl = Changelog::new();
        let mut e1 = make_entry("e1", ChangeType::Added, ChangeScope::Wallet, false);
        e1.timestamp = "2026-01-01T00:00:00Z".to_string();
        cl.add_entry(e1).unwrap();

        let mut e2 = make_entry("e2", ChangeType::Fixed, ChangeScope::Energy, false);
        e2.timestamp = "2026-03-01T00:00:00Z".to_string();
        cl.add_entry(e2).unwrap();

        let mut e3 = make_entry("e3", ChangeType::Changed, ChangeScope::UI, false);
        e3.timestamp = "2026-02-01T00:00:00Z".to_string();
        cl.add_entry(e3).unwrap();

        let recent = cl.recent_entries(2);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].id, "e2");
        assert_eq!(recent[1].id, "e3");
    }

    #[test]
    fn test_generate_markdown() {
        let mut cl = Changelog::new();
        cl.add_entry(make_entry("e1", ChangeType::Added, ChangeScope::Wallet, false))
            .unwrap();
        cl.add_entry(make_entry("e2", ChangeType::Fixed, ChangeScope::Energy, true))
            .unwrap();
        cl.tag_version("1.0.0", "First Release", vec!["e1".into()], None)
            .unwrap();

        let md = cl.generate_markdown();
        assert!(md.contains("# Changelog"));
        assert!(md.contains("## 1.0.0 - First Release"));
        assert!(md.contains("[Added][Wallet]"));
        assert!(md.contains("## Unreleased"));
        assert!(md.contains("**BREAKING**"));
    }

    #[test]
    fn test_stats() {
        let mut cl = Changelog::new();
        cl.add_entry(make_entry("e1", ChangeType::Added, ChangeScope::Wallet, true))
            .unwrap();
        cl.add_entry(make_entry("e2", ChangeType::Added, ChangeScope::Energy, false))
            .unwrap();
        cl.add_entry(make_entry("e3", ChangeType::Fixed, ChangeScope::Wallet, false))
            .unwrap();
        cl.tag_version("1.0.0", "v1", vec!["e1".into()], None)
            .unwrap();

        let stats = cl.stats();
        assert_eq!(stats.total_entries, 3);
        assert_eq!(stats.total_versions, 1);
        assert_eq!(stats.breaking_changes, 1);
        assert_eq!(*stats.by_type.get("Added").unwrap(), 2);
        assert_eq!(*stats.by_type.get("Fixed").unwrap(), 1);
        assert_eq!(*stats.by_scope.get("Wallet").unwrap(), 2);
        assert_eq!(stats.latest_version, Some("1.0.0".to_string()));
    }

    #[test]
    fn test_save_and_load() {
        let path = test_file_path("save_load");
        let mut cl = Changelog::new();
        cl.add_entry(make_entry("e1", ChangeType::Added, ChangeScope::Wallet, false))
            .unwrap();
        cl.tag_version("1.0.0", "v1", vec!["e1".into()], Some("notes".into()))
            .unwrap();

        cl.save(&path).unwrap();
        let loaded = Changelog::load(&path).unwrap();

        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.versions.len(), 1);
        assert_eq!(loaded.get_entry("e1").unwrap().id, "e1");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_or_default_missing() {
        let path = test_file_path("missing_file");
        let cl = Changelog::load_or_default(&path);
        assert!(cl.entries.is_empty());
        assert!(cl.versions.is_empty());
    }

    #[test]
    fn test_load_or_default_existing() {
        let path = test_file_path("existing_file");
        let mut cl = Changelog::new();
        cl.add_entry(make_entry("e1", ChangeType::Security, ChangeScope::Security, false))
            .unwrap();
        cl.save(&path).unwrap();

        let loaded = Changelog::load_or_default(&path);
        assert_eq!(loaded.entries.len(), 1);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_get_entry_missing() {
        let cl = Changelog::new();
        assert!(cl.get_entry("nope").is_none());
    }

    #[test]
    fn test_get_version_missing() {
        let cl = Changelog::new();
        assert!(cl.get_version("0.0.0").is_none());
    }

    #[test]
    fn test_tag_version_sets_version_field() {
        let mut cl = Changelog::new();
        cl.add_entry(make_entry("e1", ChangeType::Added, ChangeScope::Staking, false))
            .unwrap();
        assert!(cl.get_entry("e1").unwrap().version.is_none());

        cl.tag_version("2.0.0", "v2", vec!["e1".into()], None)
            .unwrap();
        assert_eq!(
            cl.get_entry("e1").unwrap().version,
            Some("2.0.0".to_string())
        );
    }
}
