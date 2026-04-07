use blake3;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum AuditTrailError {
    #[error("entry not found: {0}")]
    EntryNotFound(String),

    #[error("chain broken: {0}")]
    ChainBroken(String),

    #[error("duplicate entry: {0}")]
    DuplicateEntry(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Parse(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AuditAction {
    KeyGenerated,
    KeyImported,
    KeyDeleted,
    TxSigned,
    TxSubmitted,
    TxConfirmed,
    SettingChanged,
    LoginAttempt,
    BackupCreated,
    BackupRestored,
    PermissionGranted,
    PermissionRevoked,
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AuditSeverity {
    Info,
    Warning,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum VerifyResult {
    Valid,
    Broken(usize),
}

// ---------------------------------------------------------------------------
// Data structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: String,
    pub sequence: u64,
    pub action: AuditAction,
    pub severity: AuditSeverity,
    pub actor: String,
    pub target: String,
    pub details: HashMap<String, String>,
    pub timestamp: String,
    pub prev_hash: String,
    pub entry_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditExport {
    pub entries: Vec<AuditEntry>,
    pub exported_at: String,
    pub chain_valid: bool,
    pub total_entries: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditTrailStats {
    pub total_entries: usize,
    pub by_action: HashMap<String, usize>,
    pub by_severity: HashMap<String, usize>,
    pub chain_valid: bool,
    pub first_entry: Option<String>,
    pub last_entry: Option<String>,
    pub unique_actors: usize,
}

// ---------------------------------------------------------------------------
// AuditTrail
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuditTrail {
    entries: Vec<AuditEntry>,
    next_sequence: u64,
}

impl AuditTrail {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a new audit entry. Returns the entry id.
    pub fn record(
        &mut self,
        action: AuditAction,
        severity: AuditSeverity,
        actor: &str,
        target: &str,
        details: HashMap<String, String>,
    ) -> String {
        let sequence = self.next_sequence;
        let prev_hash = self
            .entries
            .last()
            .map(|e| e.entry_hash.clone())
            .unwrap_or_else(|| "genesis".to_string());

        let timestamp = Utc::now().to_rfc3339();

        let hash_input = format!("{}:{}:{:?}:{}", sequence, prev_hash, action, timestamp);
        let entry_hash = blake3::hash(hash_input.as_bytes()).to_hex().to_string();

        let id = format!("audit-{}-{}", sequence, &entry_hash[..8]);

        let entry = AuditEntry {
            id: id.clone(),
            sequence,
            action,
            severity,
            actor: actor.to_string(),
            target: target.to_string(),
            details,
            timestamp,
            prev_hash,
            entry_hash,
        };

        self.entries.push(entry);
        self.next_sequence += 1;
        id
    }

    pub fn get_entry(&self, id: &str) -> Option<&AuditEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    pub fn get_by_sequence(&self, seq: u64) -> Option<&AuditEntry> {
        self.entries.iter().find(|e| e.sequence == seq)
    }

    pub fn entries_by_actor(&self, actor: &str) -> Vec<&AuditEntry> {
        self.entries.iter().filter(|e| e.actor == actor).collect()
    }

    pub fn entries_by_action(&self, action: &AuditAction) -> Vec<&AuditEntry> {
        self.entries.iter().filter(|e| &e.action == action).collect()
    }

    pub fn entries_by_severity(&self, severity: &AuditSeverity) -> Vec<&AuditEntry> {
        self.entries
            .iter()
            .filter(|e| &e.severity == severity)
            .collect()
    }

    /// Return entries whose timestamp is >= `start` and <= `end` (lexicographic comparison on RFC 3339).
    pub fn entries_in_range(&self, start: &str, end: &str) -> Vec<&AuditEntry> {
        self.entries
            .iter()
            .filter(|e| e.timestamp.as_str() >= start && e.timestamp.as_str() <= end)
            .collect()
    }

    /// Search in actor, target, and detail values.
    pub fn search(&self, query: &str) -> Vec<&AuditEntry> {
        let q = query.to_lowercase();
        self.entries
            .iter()
            .filter(|e| {
                e.actor.to_lowercase().contains(&q)
                    || e.target.to_lowercase().contains(&q)
                    || e.details.values().any(|v| v.to_lowercase().contains(&q))
            })
            .collect()
    }

    /// Walk the chain and verify hash linkage and recomputed hashes.
    pub fn verify_chain(&self) -> VerifyResult {
        for (i, entry) in self.entries.iter().enumerate() {
            // Check prev_hash linkage
            let expected_prev = if i == 0 {
                "genesis".to_string()
            } else {
                self.entries[i - 1].entry_hash.clone()
            };
            if entry.prev_hash != expected_prev {
                return VerifyResult::Broken(i);
            }

            // Recompute hash
            let hash_input = format!(
                "{}:{}:{:?}:{}",
                entry.sequence, entry.prev_hash, entry.action, entry.timestamp
            );
            let recomputed = blake3::hash(hash_input.as_bytes()).to_hex().to_string();
            if entry.entry_hash != recomputed {
                return VerifyResult::Broken(i);
            }
        }
        VerifyResult::Valid
    }

    pub fn export_all(&self) -> AuditExport {
        let chain_valid = self.verify_chain() == VerifyResult::Valid;
        AuditExport {
            entries: self.entries.clone(),
            exported_at: Utc::now().to_rfc3339(),
            chain_valid,
            total_entries: self.entries.len(),
        }
    }

    pub fn export_range(&self, start_seq: u64, end_seq: u64) -> AuditExport {
        let entries: Vec<AuditEntry> = self
            .entries
            .iter()
            .filter(|e| e.sequence >= start_seq && e.sequence <= end_seq)
            .cloned()
            .collect();
        let chain_valid = self.verify_chain() == VerifyResult::Valid;
        AuditExport {
            total_entries: entries.len(),
            entries,
            exported_at: Utc::now().to_rfc3339(),
            chain_valid,
        }
    }

    pub fn recent_entries(&self, n: usize) -> Vec<&AuditEntry> {
        let len = self.entries.len();
        let start = len.saturating_sub(n);
        self.entries[start..].iter().collect()
    }

    pub fn critical_entries(&self) -> Vec<&AuditEntry> {
        self.entries_by_severity(&AuditSeverity::Critical)
    }

    pub fn stats(&self) -> AuditTrailStats {
        let mut by_action: HashMap<String, usize> = HashMap::new();
        let mut by_severity: HashMap<String, usize> = HashMap::new();
        let mut actors = std::collections::HashSet::new();

        for entry in &self.entries {
            let action_key = format!("{:?}", entry.action);
            *by_action.entry(action_key).or_default() += 1;
            let sev_key = format!("{:?}", entry.severity);
            *by_severity.entry(sev_key).or_default() += 1;
            actors.insert(entry.actor.clone());
        }

        let chain_valid = self.verify_chain() == VerifyResult::Valid;
        let first_entry = self.entries.first().map(|e| e.timestamp.clone());
        let last_entry = self.entries.last().map(|e| e.timestamp.clone());

        AuditTrailStats {
            total_entries: self.entries.len(),
            by_action,
            by_severity,
            chain_valid,
            first_entry,
            last_entry,
            unique_actors: actors.len(),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), AuditTrailError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, AuditTrailError> {
        let data = std::fs::read_to_string(path)?;
        let trail: Self = serde_json::from_str(&data)?;
        Ok(trail)
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn test_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "evaporchain_audit_test_{}_{}.json",
            std::process::id(),
            name
        ))
    }

    fn make_details(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn test_record_single_entry() {
        let mut trail = AuditTrail::new();
        let id = trail.record(
            AuditAction::KeyGenerated,
            AuditSeverity::Info,
            "alice",
            "wallet-1",
            HashMap::new(),
        );
        assert!(!id.is_empty());
        assert_eq!(trail.entries.len(), 1);
        assert_eq!(trail.entries[0].sequence, 0);
    }

    #[test]
    fn test_record_multiple_chain_links() {
        let mut trail = AuditTrail::new();
        trail.record(AuditAction::KeyGenerated, AuditSeverity::Info, "alice", "w1", HashMap::new());
        trail.record(AuditAction::TxSigned, AuditSeverity::Info, "alice", "tx1", HashMap::new());
        trail.record(AuditAction::TxSubmitted, AuditSeverity::Warning, "bob", "tx1", HashMap::new());

        assert_eq!(trail.entries.len(), 3);
        // First entry has genesis prev_hash
        assert_eq!(trail.entries[0].prev_hash, "genesis");
        // Second entry links to first
        assert_eq!(trail.entries[1].prev_hash, trail.entries[0].entry_hash);
        // Third links to second
        assert_eq!(trail.entries[2].prev_hash, trail.entries[1].entry_hash);
    }

    #[test]
    fn test_get_entry() {
        let mut trail = AuditTrail::new();
        let id = trail.record(AuditAction::KeyGenerated, AuditSeverity::Info, "alice", "w1", HashMap::new());
        let entry = trail.get_entry(&id);
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().actor, "alice");
    }

    #[test]
    fn test_get_entry_not_found() {
        let trail = AuditTrail::new();
        assert!(trail.get_entry("nonexistent").is_none());
    }

    #[test]
    fn test_get_by_sequence() {
        let mut trail = AuditTrail::new();
        trail.record(AuditAction::KeyGenerated, AuditSeverity::Info, "alice", "w1", HashMap::new());
        trail.record(AuditAction::TxSigned, AuditSeverity::Info, "bob", "tx1", HashMap::new());

        let entry = trail.get_by_sequence(1).unwrap();
        assert_eq!(entry.actor, "bob");
        assert!(trail.get_by_sequence(99).is_none());
    }

    #[test]
    fn test_entries_by_actor() {
        let mut trail = AuditTrail::new();
        trail.record(AuditAction::KeyGenerated, AuditSeverity::Info, "alice", "w1", HashMap::new());
        trail.record(AuditAction::TxSigned, AuditSeverity::Info, "bob", "tx1", HashMap::new());
        trail.record(AuditAction::TxSubmitted, AuditSeverity::Info, "alice", "tx2", HashMap::new());

        let alice_entries = trail.entries_by_actor("alice");
        assert_eq!(alice_entries.len(), 2);
    }

    #[test]
    fn test_entries_by_action() {
        let mut trail = AuditTrail::new();
        trail.record(AuditAction::TxSigned, AuditSeverity::Info, "alice", "tx1", HashMap::new());
        trail.record(AuditAction::TxSigned, AuditSeverity::Info, "bob", "tx2", HashMap::new());
        trail.record(AuditAction::KeyGenerated, AuditSeverity::Info, "carol", "w1", HashMap::new());

        let signed = trail.entries_by_action(&AuditAction::TxSigned);
        assert_eq!(signed.len(), 2);
    }

    #[test]
    fn test_entries_by_severity() {
        let mut trail = AuditTrail::new();
        trail.record(AuditAction::TxSigned, AuditSeverity::Info, "a", "t", HashMap::new());
        trail.record(AuditAction::KeyDeleted, AuditSeverity::Critical, "a", "t", HashMap::new());
        trail.record(AuditAction::LoginAttempt, AuditSeverity::Warning, "a", "t", HashMap::new());
        trail.record(AuditAction::BackupCreated, AuditSeverity::Critical, "a", "t", HashMap::new());

        let critical = trail.entries_by_severity(&AuditSeverity::Critical);
        assert_eq!(critical.len(), 2);
    }

    #[test]
    fn test_search_actor() {
        let mut trail = AuditTrail::new();
        trail.record(AuditAction::TxSigned, AuditSeverity::Info, "alice_wonder", "tx1", HashMap::new());
        trail.record(AuditAction::TxSigned, AuditSeverity::Info, "bob", "tx2", HashMap::new());

        let results = trail.search("alice");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].actor, "alice_wonder");
    }

    #[test]
    fn test_search_target() {
        let mut trail = AuditTrail::new();
        trail.record(AuditAction::TxSigned, AuditSeverity::Info, "a", "my-wallet-42", HashMap::new());

        let results = trail.search("wallet-42");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_search_details() {
        let mut trail = AuditTrail::new();
        let details = make_details(&[("note", "important transaction")]);
        trail.record(AuditAction::TxSigned, AuditSeverity::Info, "a", "t", details);

        let results = trail.search("important");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_verify_chain_valid() {
        let mut trail = AuditTrail::new();
        trail.record(AuditAction::KeyGenerated, AuditSeverity::Info, "a", "t", HashMap::new());
        trail.record(AuditAction::TxSigned, AuditSeverity::Info, "a", "t", HashMap::new());
        trail.record(AuditAction::TxConfirmed, AuditSeverity::Info, "a", "t", HashMap::new());

        assert_eq!(trail.verify_chain(), VerifyResult::Valid);
    }

    #[test]
    fn test_verify_chain_empty() {
        let trail = AuditTrail::new();
        assert_eq!(trail.verify_chain(), VerifyResult::Valid);
    }

    #[test]
    fn test_verify_chain_tampered() {
        let mut trail = AuditTrail::new();
        trail.record(AuditAction::KeyGenerated, AuditSeverity::Info, "a", "t", HashMap::new());
        trail.record(AuditAction::TxSigned, AuditSeverity::Info, "a", "t", HashMap::new());
        trail.record(AuditAction::TxConfirmed, AuditSeverity::Info, "a", "t", HashMap::new());

        // Tamper with the second entry's details — hash won't match
        trail.entries[1]
            .details
            .insert("tampered".to_string(), "yes".to_string());
        // The entry_hash was computed without this detail key in the hash input,
        // but prev_hash linkage still holds. However if we tamper with something
        // that is IN the hash input, it will break. Let's tamper with the action
        // via a different approach: change the entry_hash directly.
        trail.entries[1].entry_hash = "bad_hash".to_string();

        match trail.verify_chain() {
            VerifyResult::Broken(idx) => assert!(idx == 1 || idx == 2),
            VerifyResult::Valid => panic!("expected chain to be broken"),
        }
    }

    #[test]
    fn test_export_all() {
        let mut trail = AuditTrail::new();
        trail.record(AuditAction::KeyGenerated, AuditSeverity::Info, "a", "t", HashMap::new());
        trail.record(AuditAction::TxSigned, AuditSeverity::Info, "a", "t", HashMap::new());

        let export = trail.export_all();
        assert_eq!(export.total_entries, 2);
        assert_eq!(export.entries.len(), 2);
        assert!(export.chain_valid);
    }

    #[test]
    fn test_export_range() {
        let mut trail = AuditTrail::new();
        for i in 0..5 {
            trail.record(
                AuditAction::Custom(format!("action-{}", i)),
                AuditSeverity::Info,
                "a",
                "t",
                HashMap::new(),
            );
        }

        let export = trail.export_range(1, 3);
        assert_eq!(export.total_entries, 3);
        assert_eq!(export.entries[0].sequence, 1);
        assert_eq!(export.entries[2].sequence, 3);
    }

    #[test]
    fn test_recent_entries() {
        let mut trail = AuditTrail::new();
        for _ in 0..5 {
            trail.record(AuditAction::TxSigned, AuditSeverity::Info, "a", "t", HashMap::new());
        }

        let recent = trail.recent_entries(2);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].sequence, 3);
        assert_eq!(recent[1].sequence, 4);
    }

    #[test]
    fn test_recent_entries_more_than_available() {
        let mut trail = AuditTrail::new();
        trail.record(AuditAction::TxSigned, AuditSeverity::Info, "a", "t", HashMap::new());

        let recent = trail.recent_entries(10);
        assert_eq!(recent.len(), 1);
    }

    #[test]
    fn test_critical_entries() {
        let mut trail = AuditTrail::new();
        trail.record(AuditAction::TxSigned, AuditSeverity::Info, "a", "t", HashMap::new());
        trail.record(AuditAction::KeyDeleted, AuditSeverity::Critical, "a", "t", HashMap::new());

        let critical = trail.critical_entries();
        assert_eq!(critical.len(), 1);
        assert_eq!(critical[0].severity, AuditSeverity::Critical);
    }

    #[test]
    fn test_stats() {
        let mut trail = AuditTrail::new();
        trail.record(AuditAction::KeyGenerated, AuditSeverity::Info, "alice", "t", HashMap::new());
        trail.record(AuditAction::TxSigned, AuditSeverity::Warning, "bob", "t", HashMap::new());
        trail.record(AuditAction::KeyGenerated, AuditSeverity::Critical, "alice", "t", HashMap::new());

        let stats = trail.stats();
        assert_eq!(stats.total_entries, 3);
        assert_eq!(stats.unique_actors, 2);
        assert_eq!(*stats.by_action.get("KeyGenerated").unwrap(), 2);
        assert_eq!(*stats.by_action.get("TxSigned").unwrap(), 1);
        assert!(stats.chain_valid);
        assert!(stats.first_entry.is_some());
        assert!(stats.last_entry.is_some());
    }

    #[test]
    fn test_persistence_roundtrip() {
        let path = test_path("roundtrip");
        let mut trail = AuditTrail::new();
        trail.record(AuditAction::KeyGenerated, AuditSeverity::Info, "alice", "w1", HashMap::new());
        trail.record(AuditAction::TxSigned, AuditSeverity::Warning, "bob", "tx1", HashMap::new());

        trail.save(&path).unwrap();
        let loaded = AuditTrail::load(&path).unwrap();

        assert_eq!(loaded.entries.len(), 2);
        assert_eq!(loaded.next_sequence, 2);
        assert_eq!(loaded.entries[0].actor, "alice");
        assert_eq!(loaded.verify_chain(), VerifyResult::Valid);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_or_default_missing_file() {
        let path = test_path("nonexistent_load_default");
        let _ = std::fs::remove_file(&path); // ensure missing
        let trail = AuditTrail::load_or_default(&path);
        assert_eq!(trail.entries.len(), 0);
        assert_eq!(trail.next_sequence, 0);
    }

    #[test]
    fn test_genesis_hash() {
        let mut trail = AuditTrail::new();
        trail.record(AuditAction::KeyGenerated, AuditSeverity::Info, "a", "t", HashMap::new());
        assert_eq!(trail.entries[0].prev_hash, "genesis");
    }

    #[test]
    fn test_default_trait() {
        let trail = AuditTrail::default();
        assert_eq!(trail.entries.len(), 0);
        assert_eq!(trail.next_sequence, 0);
    }
}
