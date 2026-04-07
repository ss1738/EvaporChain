//! Wallet state snapshot system — capture, compare, and restore full wallet state.
//!
//! Provides persistent snapshots of all wallet state entries with BLAKE3 integrity
//! checksums. Supports diffing between snapshots, date-range queries, label search,
//! and automatic pruning to stay within a configurable retention limit.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use thiserror::Error;

static SNAP_COUNTER: AtomicU64 = AtomicU64::new(0);

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum SnapshotError {
    #[error("snapshot not found: {0}")]
    NotFound(String),
    #[error("snapshot limit reached: {0}")]
    LimitReached(usize),
    #[error("invalid checksum for snapshot: {0}")]
    InvalidChecksum(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON parse error: {0}")]
    Parse(#[from] serde_json::Error),
}

// ──────────────────────────── StateEntry ──────────────────────────────

/// A single key-value entry in the wallet state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StateEntry {
    /// Unique key identifying this piece of state.
    pub key: String,
    /// Serialised value.
    pub value: String,
    /// Category tag (e.g. "balance", "config", "contacts", "tokens", "objects").
    pub category: String,
}

// ──────────────────────────── Snapshot ─────────────────────────────────

/// A point-in-time capture of the full wallet state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// Unique identifier (e.g. "snap_1712345678901").
    pub id: String,
    /// Human-readable label.
    pub label: String,
    /// ISO 8601 creation timestamp.
    pub created_at: String,
    /// State entries keyed by their `key` field.
    pub entries: HashMap<String, StateEntry>,
    /// BLAKE3 checksum of sorted "key=value\n" pairs.
    pub checksum: String,
    /// Approximate size in bytes of all entry values.
    pub size_bytes: usize,
    /// Arbitrary metadata (e.g. "source" → "auto", "version" → "2").
    pub metadata: HashMap<String, String>,
}

impl Snapshot {
    /// Create a new empty snapshot.
    pub fn new(id: &str, label: &str) -> Self {
        let mut snap = Self {
            id: id.to_string(),
            label: label.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            entries: HashMap::new(),
            checksum: String::new(),
            size_bytes: 0,
            metadata: HashMap::new(),
        };
        snap.checksum = snap.compute_checksum();
        snap
    }

    /// Add or replace a state entry, recomputing checksum and size.
    pub fn add_entry(&mut self, key: &str, value: &str, category: &str) {
        self.entries.insert(
            key.to_string(),
            StateEntry {
                key: key.to_string(),
                value: value.to_string(),
                category: category.to_string(),
            },
        );
        self.recompute();
    }

    /// Remove an entry by key, recomputing checksum and size.
    pub fn remove_entry(&mut self, key: &str) -> Option<StateEntry> {
        let removed = self.entries.remove(key);
        if removed.is_some() {
            self.recompute();
        }
        removed
    }

    /// Look up an entry by key.
    pub fn get_entry(&self, key: &str) -> Option<&StateEntry> {
        self.entries.get(key)
    }

    /// Return all entries matching a given category.
    pub fn entries_by_category(&self, category: &str) -> Vec<&StateEntry> {
        self.entries
            .values()
            .filter(|e| e.category == category)
            .collect()
    }

    /// Number of entries in this snapshot.
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// Compute the BLAKE3 checksum of all entries sorted by key.
    pub fn compute_checksum(&self) -> String {
        let mut keys: Vec<&String> = self.entries.keys().collect();
        keys.sort();
        let mut data = String::new();
        for k in keys {
            if let Some(entry) = self.entries.get(k) {
                data.push_str(&format!("{}={}\n", k, entry.value));
            }
        }
        blake3::hash(data.as_bytes()).to_hex().to_string()
    }

    /// Verify that the stored checksum matches the recomputed one.
    pub fn verify(&self) -> bool {
        self.checksum == self.compute_checksum()
    }

    /// Builder-style method to add metadata.
    pub fn with_metadata(mut self, key: &str, value: &str) -> Self {
        self.metadata.insert(key.to_string(), value.to_string());
        self
    }

    /// Recompute checksum and size after mutation.
    fn recompute(&mut self) {
        self.checksum = self.compute_checksum();
        self.size_bytes = self
            .entries
            .values()
            .map(|e| e.key.len() + e.value.len() + e.category.len())
            .sum();
    }
}

// ──────────────────────────── SnapshotDiff ────────────────────────────

/// The difference between two snapshots.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotDiff {
    /// Source snapshot id.
    pub from_id: String,
    /// Target snapshot id.
    pub to_id: String,
    /// Keys present only in the target.
    pub added: Vec<String>,
    /// Keys present only in the source.
    pub removed: Vec<String>,
    /// Keys present in both but with different values: (key, old_value, new_value).
    pub changed: Vec<(String, String, String)>,
    /// Number of keys that are identical in both.
    pub unchanged: usize,
}

impl SnapshotDiff {
    /// Total number of changes (added + removed + changed).
    pub fn total_changes(&self) -> usize {
        self.added.len() + self.removed.len() + self.changed.len()
    }

    /// Whether there are any differences at all.
    pub fn has_changes(&self) -> bool {
        self.total_changes() > 0
    }

    /// Human-readable summary string.
    pub fn summary(&self) -> String {
        format!(
            "+{} -{} ~{} ({} unchanged)",
            self.added.len(),
            self.removed.len(),
            self.changed.len(),
            self.unchanged,
        )
    }
}

// ──────────────────────────── SnapshotStore ───────────────────────────

/// Manages an ordered collection of wallet state snapshots.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotStore {
    /// Snapshots in chronological order (oldest first).
    pub snapshots: Vec<Snapshot>,
    /// Maximum number of snapshots to retain.
    pub max_snapshots: usize,
}

impl Default for SnapshotStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SnapshotStore {
    /// Create an empty store with default limits.
    pub fn new() -> Self {
        Self {
            snapshots: Vec::new(),
            max_snapshots: 50,
        }
    }

    /// Builder-style method to set the maximum snapshot count.
    pub fn with_max(mut self, max: usize) -> Self {
        self.max_snapshots = max;
        self
    }

    /// Capture a new snapshot from a list of (key, value, category) tuples.
    ///
    /// Auto-generates an id as `snap_{timestamp_millis}`, auto-prunes the oldest
    /// snapshots if the store exceeds its limit, and returns the new snapshot id.
    pub fn capture(
        &mut self,
        label: &str,
        entries: Vec<(String, String, String)>,
    ) -> String {
        let now = chrono::Utc::now();
        let seq = SNAP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let id = format!("snap_{}_{}", now.timestamp_millis(), seq);
        let mut snap = Snapshot::new(&id, label);
        // Overwrite created_at to use the same instant.
        snap.created_at = now.to_rfc3339();
        for (key, value, category) in entries {
            snap.add_entry(&key, &value, &category);
        }
        self.snapshots.push(snap);
        // Auto-prune if over limit.
        while self.snapshots.len() > self.max_snapshots {
            self.snapshots.remove(0);
        }
        id
    }

    /// Get a snapshot by id.
    pub fn get(&self, id: &str) -> Option<&Snapshot> {
        self.snapshots.iter().find(|s| s.id == id)
    }

    /// Get the most recent snapshot.
    pub fn latest(&self) -> Option<&Snapshot> {
        self.snapshots.last()
    }

    /// List all snapshots (oldest first).
    pub fn list(&self) -> &[Snapshot] {
        &self.snapshots
    }

    /// Remove a snapshot by id.
    pub fn remove(&mut self, id: &str) -> Result<Snapshot, SnapshotError> {
        let idx = self
            .snapshots
            .iter()
            .position(|s| s.id == id)
            .ok_or_else(|| SnapshotError::NotFound(id.to_string()))?;
        Ok(self.snapshots.remove(idx))
    }

    /// Compute the diff between two snapshots.
    pub fn diff(&self, from_id: &str, to_id: &str) -> Result<SnapshotDiff, SnapshotError> {
        let from = self
            .get(from_id)
            .ok_or_else(|| SnapshotError::NotFound(from_id.to_string()))?;
        let to = self
            .get(to_id)
            .ok_or_else(|| SnapshotError::NotFound(to_id.to_string()))?;

        let mut added = Vec::new();
        let mut removed = Vec::new();
        let mut changed = Vec::new();
        let mut unchanged: usize = 0;

        // Keys in `from` but not in `to` → removed; keys in both → unchanged or changed.
        for (key, from_entry) in &from.entries {
            match to.entries.get(key) {
                None => removed.push(key.clone()),
                Some(to_entry) => {
                    if from_entry.value != to_entry.value {
                        changed.push((
                            key.clone(),
                            from_entry.value.clone(),
                            to_entry.value.clone(),
                        ));
                    } else {
                        unchanged += 1;
                    }
                }
            }
        }
        // Keys in `to` but not in `from` → added.
        for key in to.entries.keys() {
            if !from.entries.contains_key(key) {
                added.push(key.clone());
            }
        }

        added.sort();
        removed.sort();
        changed.sort_by(|a, b| a.0.cmp(&b.0));

        Ok(SnapshotDiff {
            from_id: from_id.to_string(),
            to_id: to_id.to_string(),
            added,
            removed,
            changed,
            unchanged,
        })
    }

    /// Restore (clone) all entries from a snapshot.
    pub fn restore_entries(&self, id: &str) -> Result<Vec<StateEntry>, SnapshotError> {
        let snap = self
            .get(id)
            .ok_or_else(|| SnapshotError::NotFound(id.to_string()))?;
        Ok(snap.entries.values().cloned().collect())
    }

    /// Search snapshots by label (case-insensitive substring match).
    pub fn search(&self, label_query: &str) -> Vec<&Snapshot> {
        let query = label_query.to_lowercase();
        self.snapshots
            .iter()
            .filter(|s| s.label.to_lowercase().contains(&query))
            .collect()
    }

    /// Filter snapshots whose `created_at` falls within [from, to] (lexicographic / RFC 3339).
    pub fn by_date_range(&self, from: &str, to: &str) -> Vec<&Snapshot> {
        self.snapshots
            .iter()
            .filter(|s| s.created_at.as_str() >= from && s.created_at.as_str() <= to)
            .collect()
    }

    /// Sum of `size_bytes` across all stored snapshots.
    pub fn total_size(&self) -> usize {
        self.snapshots.iter().map(|s| s.size_bytes).sum()
    }

    /// Remove the oldest snapshots, keeping at most `keep`. Returns the number removed.
    pub fn prune_oldest(&mut self, keep: usize) -> usize {
        if self.snapshots.len() <= keep {
            return 0;
        }
        let remove_count = self.snapshots.len() - keep;
        self.snapshots.drain(..remove_count);
        remove_count
    }

    // ── Persistence ──────────────────────────────────────────────

    /// Save the store to a JSON file.
    pub fn save(&self, path: &Path) -> Result<(), SnapshotError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Load the store from a JSON file.
    pub fn load(path: &Path) -> Result<Self, SnapshotError> {
        let data = std::fs::read_to_string(path)?;
        let store: Self = serde_json::from_str(&data)?;
        Ok(store)
    }

    /// Load the store from a JSON file, falling back to an empty default.
    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ──────────────────────────── Tests ───────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build a simple entries vec.
    fn sample_entries() -> Vec<(String, String, String)> {
        vec![
            ("balance_evap".into(), "1000".into(), "balance".into()),
            ("balance_usdt".into(), "250".into(), "balance".into()),
            ("config_theme".into(), "light".into(), "config".into()),
        ]
    }

    fn test_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("snapshot_test_{}_{}", name, std::process::id()))
    }

    #[test]
    fn test_capture_snapshot() {
        let mut store = SnapshotStore::new();
        let id = store.capture("initial", sample_entries());
        assert!(id.starts_with("snap_"));
        assert_eq!(store.list().len(), 1);
        let snap = store.get(&id).unwrap();
        assert_eq!(snap.label, "initial");
        assert_eq!(snap.entry_count(), 3);
    }

    #[test]
    fn test_snapshot_checksum_valid() {
        let mut snap = Snapshot::new("s1", "test");
        snap.add_entry("a", "1", "balance");
        snap.add_entry("b", "2", "balance");
        assert!(!snap.checksum.is_empty());
        assert_eq!(snap.checksum, snap.compute_checksum());
    }

    #[test]
    fn test_snapshot_checksum_changes_on_modify() {
        let mut snap = Snapshot::new("s1", "test");
        snap.add_entry("a", "1", "balance");
        let c1 = snap.checksum.clone();
        snap.add_entry("b", "2", "balance");
        assert_ne!(c1, snap.checksum);
    }

    #[test]
    fn test_snapshot_verify() {
        let mut snap = Snapshot::new("s1", "test");
        snap.add_entry("x", "42", "config");
        assert!(snap.verify());
        // Tamper with the checksum.
        snap.checksum = "bad".to_string();
        assert!(!snap.verify());
    }

    #[test]
    fn test_add_remove_entry() {
        let mut snap = Snapshot::new("s1", "test");
        snap.add_entry("k1", "v1", "tokens");
        assert_eq!(snap.entry_count(), 1);
        let removed = snap.remove_entry("k1");
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().value, "v1");
        assert_eq!(snap.entry_count(), 0);
        assert!(snap.verify());
    }

    #[test]
    fn test_entries_by_category() {
        let mut snap = Snapshot::new("s1", "test");
        snap.add_entry("a", "1", "balance");
        snap.add_entry("b", "2", "config");
        snap.add_entry("c", "3", "balance");
        let balances = snap.entries_by_category("balance");
        assert_eq!(balances.len(), 2);
        let configs = snap.entries_by_category("config");
        assert_eq!(configs.len(), 1);
    }

    #[test]
    fn test_diff_no_changes() {
        let mut store = SnapshotStore::new();
        let entries = sample_entries();
        let id1 = store.capture("v1", entries.clone());
        let id2 = store.capture("v2", entries);
        let diff = store.diff(&id1, &id2).unwrap();
        assert!(!diff.has_changes());
        assert_eq!(diff.unchanged, 3);
    }

    #[test]
    fn test_diff_with_additions() {
        let mut store = SnapshotStore::new();
        let id1 = store.capture("v1", vec![("a".into(), "1".into(), "b".into())]);
        let id2 = store.capture("v2", vec![
            ("a".into(), "1".into(), "b".into()),
            ("b".into(), "2".into(), "b".into()),
        ]);
        let diff = store.diff(&id1, &id2).unwrap();
        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.added[0], "b");
        assert_eq!(diff.removed.len(), 0);
    }

    #[test]
    fn test_diff_with_removals() {
        let mut store = SnapshotStore::new();
        let id1 = store.capture("v1", vec![
            ("a".into(), "1".into(), "b".into()),
            ("b".into(), "2".into(), "b".into()),
        ]);
        let id2 = store.capture("v2", vec![("a".into(), "1".into(), "b".into())]);
        let diff = store.diff(&id1, &id2).unwrap();
        assert_eq!(diff.removed.len(), 1);
        assert_eq!(diff.removed[0], "b");
    }

    #[test]
    fn test_diff_with_changes() {
        let mut store = SnapshotStore::new();
        let id1 = store.capture("v1", vec![("a".into(), "1".into(), "b".into())]);
        let id2 = store.capture("v2", vec![("a".into(), "99".into(), "b".into())]);
        let diff = store.diff(&id1, &id2).unwrap();
        assert_eq!(diff.changed.len(), 1);
        assert_eq!(diff.changed[0], ("a".into(), "1".into(), "99".into()));
    }

    #[test]
    fn test_diff_mixed() {
        let mut store = SnapshotStore::new();
        let id1 = store.capture("v1", vec![
            ("keep".into(), "same".into(), "c".into()),
            ("change".into(), "old".into(), "c".into()),
            ("remove".into(), "gone".into(), "c".into()),
        ]);
        let id2 = store.capture("v2", vec![
            ("keep".into(), "same".into(), "c".into()),
            ("change".into(), "new".into(), "c".into()),
            ("added".into(), "fresh".into(), "c".into()),
        ]);
        let diff = store.diff(&id1, &id2).unwrap();
        assert_eq!(diff.added, vec!["added"]);
        assert_eq!(diff.removed, vec!["remove"]);
        assert_eq!(diff.changed.len(), 1);
        assert_eq!(diff.unchanged, 1);
        assert!(diff.has_changes());
        assert_eq!(diff.total_changes(), 3);
    }

    #[test]
    fn test_diff_not_found() {
        let store = SnapshotStore::new();
        let result = store.diff("nope", "also_nope");
        assert!(result.is_err());
        match result.unwrap_err() {
            SnapshotError::NotFound(id) => assert_eq!(id, "nope"),
            other => panic!("expected NotFound, got {:?}", other),
        }
    }

    #[test]
    fn test_latest() {
        let mut store = SnapshotStore::new();
        assert!(store.latest().is_none());
        store.capture("first", sample_entries());
        let id2 = store.capture("second", sample_entries());
        assert_eq!(store.latest().unwrap().id, id2);
    }

    #[test]
    fn test_remove_snapshot() {
        let mut store = SnapshotStore::new();
        let id = store.capture("rm-me", sample_entries());
        let removed = store.remove(&id).unwrap();
        assert_eq!(removed.label, "rm-me");
        assert!(store.list().is_empty());
    }

    #[test]
    fn test_remove_not_found() {
        let mut store = SnapshotStore::new();
        let result = store.remove("nonexistent");
        assert!(result.is_err());
        match result.unwrap_err() {
            SnapshotError::NotFound(_) => {}
            other => panic!("expected NotFound, got {:?}", other),
        }
    }

    #[test]
    fn test_restore_entries() {
        let mut store = SnapshotStore::new();
        let id = store.capture("restore-test", sample_entries());
        let entries = store.restore_entries(&id).unwrap();
        assert_eq!(entries.len(), 3);
        // All entries should be clones with correct data.
        let keys: Vec<String> = entries.iter().map(|e| e.key.clone()).collect();
        assert!(keys.contains(&"balance_evap".to_string()));
    }

    #[test]
    fn test_auto_prune_at_max() {
        let mut store = SnapshotStore::new().with_max(3);
        for i in 0..5 {
            store.capture(&format!("snap-{}", i), sample_entries());
        }
        assert_eq!(store.list().len(), 3);
        // The oldest two should have been pruned; labels 2, 3, 4 remain.
        let labels: Vec<&str> = store.list().iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, vec!["snap-2", "snap-3", "snap-4"]);
    }

    #[test]
    fn test_prune_oldest() {
        let mut store = SnapshotStore::new();
        for i in 0..5 {
            store.capture(&format!("s{}", i), sample_entries());
        }
        let removed = store.prune_oldest(2);
        assert_eq!(removed, 3);
        assert_eq!(store.list().len(), 2);
        assert_eq!(store.list()[0].label, "s3");
    }

    #[test]
    fn test_search_by_label() {
        let mut store = SnapshotStore::new();
        store.capture("Daily Backup", sample_entries());
        store.capture("weekly backup", sample_entries());
        store.capture("migration", sample_entries());
        let results = store.search("backup");
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_total_size() {
        let mut store = SnapshotStore::new();
        let id = store.capture("sized", sample_entries());
        let snap_size = store.get(&id).unwrap().size_bytes;
        assert!(snap_size > 0);
        assert_eq!(store.total_size(), snap_size);
    }

    #[test]
    fn test_with_metadata() {
        let snap = Snapshot::new("m1", "meta-test")
            .with_metadata("source", "auto")
            .with_metadata("version", "3");
        assert_eq!(snap.metadata.get("source").unwrap(), "auto");
        assert_eq!(snap.metadata.get("version").unwrap(), "3");
    }

    #[test]
    fn test_persistence_roundtrip() {
        let path = test_path("roundtrip");
        let mut store = SnapshotStore::new();
        store.capture("persist-test", sample_entries());
        store.save(&path).unwrap();

        let loaded = SnapshotStore::load(&path).unwrap();
        assert_eq!(loaded.list().len(), 1);
        assert_eq!(loaded.list()[0].label, "persist-test");
        assert!(loaded.list()[0].verify());

        // Cleanup.
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_snapshot_summary() {
        let diff = SnapshotDiff {
            from_id: "a".into(),
            to_id: "b".into(),
            added: vec!["x".into(), "y".into()],
            removed: vec!["z".into()],
            changed: vec![("w".into(), "old".into(), "new".into())],
            unchanged: 5,
        };
        assert_eq!(diff.summary(), "+2 -1 ~1 (5 unchanged)");
    }
}
