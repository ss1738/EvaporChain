//! Tamper-evident audit log — every wallet action recorded with BLAKE3 hash chain.
//!
//! Each entry contains the BLAKE3 hash of the previous entry, forming an
//! append-only chain. Any modification to a past entry breaks the chain,
//! making tampering detectable. Designed for institutional compliance.

use std::path::Path;

use serde::{Deserialize, Serialize};

// ──────────────────────────── Types ──────────────────────────────────────

#[derive(Debug, Clone, thiserror::Error)]
pub enum AuditError {
    #[error("chain integrity violated at entry {0}")]
    IntegrityViolation(usize),
    #[error("empty log")]
    EmptyLog,
    #[error("io error: {0}")]
    Io(String),
    #[error("json error: {0}")]
    Json(String),
}

impl From<std::io::Error> for AuditError {
    fn from(e: std::io::Error) -> Self {
        AuditError::Io(e.to_string())
    }
}
impl From<serde_json::Error> for AuditError {
    fn from(e: serde_json::Error) -> Self {
        AuditError::Json(e.to_string())
    }
}

/// Category of auditable action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditAction {
    // Account
    AccountCreate,
    AccountSwitch,
    AccountImport,
    // Transfers
    TransferSend,
    TransferReceive,
    // Objects / Energy
    ObjectCreate,
    ObjectRefresh,
    ObjectEvaporate,
    // Staking
    StakeDeposit,
    StakeWithdraw,
    RewardClaim,
    // Governance
    VoteCast,
    ProposalCreate,
    // NFT / Token
    NftMint,
    NftTransfer,
    TokenDeploy,
    TokenTransfer,
    // Security
    PasswordChange,
    BackupCreate,
    BackupRestore,
    KeyRotation,
    // Policy
    SpendingPolicyChange,
    MultisigApproval,
    // Session
    SessionCreate,
    SessionRevoke,
    DappConnect,
    DappDisconnect,
    // Bridge
    BridgeInitiate,
    BridgeComplete,
    // Config
    ConfigChange,
    // Generic
    Custom(String),
}

impl AuditAction {
    pub fn label(&self) -> String {
        match self {
            AuditAction::AccountCreate => "account.create".into(),
            AuditAction::AccountSwitch => "account.switch".into(),
            AuditAction::AccountImport => "account.import".into(),
            AuditAction::TransferSend => "transfer.send".into(),
            AuditAction::TransferReceive => "transfer.receive".into(),
            AuditAction::ObjectCreate => "object.create".into(),
            AuditAction::ObjectRefresh => "object.refresh".into(),
            AuditAction::ObjectEvaporate => "object.evaporate".into(),
            AuditAction::StakeDeposit => "stake.deposit".into(),
            AuditAction::StakeWithdraw => "stake.withdraw".into(),
            AuditAction::RewardClaim => "stake.claim".into(),
            AuditAction::VoteCast => "governance.vote".into(),
            AuditAction::ProposalCreate => "governance.propose".into(),
            AuditAction::NftMint => "nft.mint".into(),
            AuditAction::NftTransfer => "nft.transfer".into(),
            AuditAction::TokenDeploy => "token.deploy".into(),
            AuditAction::TokenTransfer => "token.transfer".into(),
            AuditAction::PasswordChange => "security.password".into(),
            AuditAction::BackupCreate => "security.backup".into(),
            AuditAction::BackupRestore => "security.restore".into(),
            AuditAction::KeyRotation => "security.rotate".into(),
            AuditAction::SpendingPolicyChange => "policy.spending".into(),
            AuditAction::MultisigApproval => "policy.multisig".into(),
            AuditAction::SessionCreate => "session.create".into(),
            AuditAction::SessionRevoke => "session.revoke".into(),
            AuditAction::DappConnect => "dapp.connect".into(),
            AuditAction::DappDisconnect => "dapp.disconnect".into(),
            AuditAction::BridgeInitiate => "bridge.initiate".into(),
            AuditAction::BridgeComplete => "bridge.complete".into(),
            AuditAction::ConfigChange => "config.change".into(),
            AuditAction::Custom(s) => format!("custom.{}", s),
        }
    }
}

/// Severity level for audit entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

/// A single audit log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    /// Sequential index (0-based).
    pub index: u64,
    /// Timestamp (RFC3339).
    pub timestamp: String,
    /// Action performed.
    pub action: AuditAction,
    /// Severity.
    pub severity: Severity,
    /// Account that performed the action.
    pub account: String,
    /// Human-readable description.
    pub description: String,
    /// Key-value metadata.
    pub metadata: std::collections::HashMap<String, String>,
    /// BLAKE3 hash of the previous entry (hex). Genesis entry uses all zeros.
    pub prev_hash: String,
    /// BLAKE3 hash of this entry's content (hex).
    pub hash: String,
}

impl AuditEntry {
    /// Compute the hash of this entry's content (excluding the hash field itself).
    fn compute_hash(&self) -> String {
        let content = format!(
            "{}|{}|{}|{:?}|{}|{}|{:?}|{}",
            self.index,
            self.timestamp,
            self.action.label(),
            self.severity,
            self.account,
            self.description,
            self.metadata,
            self.prev_hash
        );
        blake3::hash(content.as_bytes()).to_hex().to_string()
    }

    /// Verify this entry's hash is correct.
    pub fn verify(&self) -> bool {
        self.hash == self.compute_hash()
    }
}

// ──────────────────────────── Log ────────────────────────────────────────

/// The audit log — append-only, hash-chained.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditLog {
    pub entries: Vec<AuditEntry>,
    /// Maximum entries to retain (0 = unlimited).
    pub max_entries: usize,
}

impl AuditLog {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            max_entries: 0,
        }
    }

    pub fn with_max_entries(max: usize) -> Self {
        Self {
            entries: Vec::new(),
            max_entries: max,
        }
    }

    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, AuditError> {
        let data = std::fs::read_to_string(path)?;
        let log: AuditLog = serde_json::from_str(&data)?;
        Ok(log)
    }

    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), AuditError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Append a new entry to the log.
    pub fn append(
        &mut self,
        action: AuditAction,
        severity: Severity,
        account: &str,
        description: &str,
        metadata: std::collections::HashMap<String, String>,
    ) -> &AuditEntry {
        let index = self.entries.len() as u64;
        let prev_hash = self
            .entries
            .last()
            .map(|e| e.hash.clone())
            .unwrap_or_else(|| "0".repeat(64)); // genesis

        let mut entry = AuditEntry {
            index,
            timestamp: chrono::Utc::now().to_rfc3339(),
            action,
            severity,
            account: account.to_string(),
            description: description.to_string(),
            metadata,
            prev_hash,
            hash: String::new(),
        };
        entry.hash = entry.compute_hash();

        self.entries.push(entry);

        // Evict oldest if over capacity (but keep at least 1 for chain continuity)
        if self.max_entries > 0 && self.entries.len() > self.max_entries {
            self.entries.remove(0);
        }

        self.entries.last().unwrap()
    }

    /// Convenience: log an info-level action.
    pub fn info(&mut self, action: AuditAction, account: &str, description: &str) -> &AuditEntry {
        self.append(
            action,
            Severity::Info,
            account,
            description,
            std::collections::HashMap::new(),
        )
    }

    /// Convenience: log a warning-level action.
    pub fn warn(&mut self, action: AuditAction, account: &str, description: &str) -> &AuditEntry {
        self.append(
            action,
            Severity::Warning,
            account,
            description,
            std::collections::HashMap::new(),
        )
    }

    /// Convenience: log a critical-level action.
    pub fn critical(
        &mut self,
        action: AuditAction,
        account: &str,
        description: &str,
    ) -> &AuditEntry {
        self.append(
            action,
            Severity::Critical,
            account,
            description,
            std::collections::HashMap::new(),
        )
    }

    /// Verify the entire chain. Returns Ok(()) if intact, Err with first broken index.
    pub fn verify_chain(&self) -> Result<(), AuditError> {
        if self.entries.is_empty() {
            return Ok(());
        }

        for (i, entry) in self.entries.iter().enumerate() {
            // Verify self-hash
            if !entry.verify() {
                return Err(AuditError::IntegrityViolation(i));
            }

            // Verify chain link
            if i == 0 {
                // First entry in storage — prev_hash might be genesis or from evicted entry
                // Only verify if it's supposed to be genesis (index == 0)
                if entry.index == 0 && entry.prev_hash != "0".repeat(64) {
                    return Err(AuditError::IntegrityViolation(0));
                }
            } else {
                if entry.prev_hash != self.entries[i - 1].hash {
                    return Err(AuditError::IntegrityViolation(i));
                }
            }
        }
        Ok(())
    }

    /// Get the latest entry.
    pub fn latest(&self) -> Option<&AuditEntry> {
        self.entries.last()
    }

    /// Get entry by index.
    pub fn get(&self, index: u64) -> Option<&AuditEntry> {
        self.entries.iter().find(|e| e.index == index)
    }

    /// Filter by action type.
    pub fn filter_action(&self, action: &AuditAction) -> Vec<&AuditEntry> {
        self.entries
            .iter()
            .filter(|e| &e.action == action)
            .collect()
    }

    /// Filter by severity.
    pub fn filter_severity(&self, min: Severity) -> Vec<&AuditEntry> {
        self.entries.iter().filter(|e| e.severity >= min).collect()
    }

    /// Filter by account.
    pub fn filter_account(&self, account: &str) -> Vec<&AuditEntry> {
        let acc = account.to_lowercase();
        self.entries
            .iter()
            .filter(|e| e.account.to_lowercase() == acc)
            .collect()
    }

    /// Recent entries.
    pub fn recent(&self, count: usize) -> Vec<&AuditEntry> {
        self.entries.iter().rev().take(count).collect()
    }

    /// Search description text.
    pub fn search(&self, query: &str) -> Vec<&AuditEntry> {
        let q = query.to_lowercase();
        self.entries
            .iter()
            .filter(|e| e.description.to_lowercase().contains(&q) || e.action.label().contains(&q))
            .collect()
    }

    /// Total entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Export log as CSV string.
    pub fn to_csv(&self) -> String {
        let mut csv = String::from("index,timestamp,action,severity,account,description,hash\n");
        for e in &self.entries {
            csv.push_str(&format!(
                "{},{},{},{:?},{},{},{}\n",
                e.index,
                e.timestamp,
                e.action.label(),
                e.severity,
                e.account,
                e.description.replace(',', ";"),
                &e.hash[..16]
            ));
        }
        csv
    }
}

impl Default for AuditLog {
    fn default() -> Self {
        Self::new()
    }
}

/// Default path.
pub fn default_audit_path() -> std::path::PathBuf {
    crate::config::default_data_dir().join("audit_log.json")
}

// ──────────────────────────── Tests ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_log() -> AuditLog {
        let mut log = AuditLog::new();
        log.info(AuditAction::AccountCreate, "alice", "Created account alice");
        log.info(
            AuditAction::TransferSend,
            "alice",
            "Sent 1000 EVAP to 0xbob",
        );
        log.warn(
            AuditAction::SpendingPolicyChange,
            "alice",
            "Daily limit changed to 50000",
        );
        log.critical(
            AuditAction::PasswordChange,
            "alice",
            "Master password changed",
        );
        log
    }

    #[test]
    fn test_append() {
        let log = make_log();
        assert_eq!(log.len(), 4);
    }

    #[test]
    fn test_hash_chain_valid() {
        let log = make_log();
        assert!(log.verify_chain().is_ok());
    }

    #[test]
    fn test_genesis_entry() {
        let log = make_log();
        let genesis = &log.entries[0];
        assert_eq!(genesis.index, 0);
        assert_eq!(genesis.prev_hash, "0".repeat(64));
    }

    #[test]
    fn test_chain_links() {
        let log = make_log();
        for i in 1..log.entries.len() {
            assert_eq!(log.entries[i].prev_hash, log.entries[i - 1].hash);
        }
    }

    #[test]
    fn test_entry_verify() {
        let log = make_log();
        for entry in &log.entries {
            assert!(entry.verify());
        }
    }

    #[test]
    fn test_tamper_detection() {
        let mut log = make_log();
        // Tamper with an entry
        log.entries[1].description = "TAMPERED".to_string();
        assert!(log.verify_chain().is_err());
        let err = log.verify_chain().unwrap_err();
        match err {
            AuditError::IntegrityViolation(idx) => assert_eq!(idx, 1),
            _ => panic!("expected IntegrityViolation"),
        }
    }

    #[test]
    fn test_chain_break_detection() {
        let mut log = make_log();
        // Break the chain by changing a hash
        log.entries[1].prev_hash = "bad_hash".to_string();
        assert!(log.verify_chain().is_err());
    }

    #[test]
    fn test_filter_action() {
        let log = make_log();
        let transfers = log.filter_action(&AuditAction::TransferSend);
        assert_eq!(transfers.len(), 1);
    }

    #[test]
    fn test_filter_severity() {
        let log = make_log();
        let warnings = log.filter_severity(Severity::Warning);
        assert_eq!(warnings.len(), 2); // Warning + Critical
        let critical = log.filter_severity(Severity::Critical);
        assert_eq!(critical.len(), 1);
    }

    #[test]
    fn test_filter_account() {
        let log = make_log();
        let alice = log.filter_account("alice");
        assert_eq!(alice.len(), 4);
        let bob = log.filter_account("bob");
        assert_eq!(bob.len(), 0);
    }

    #[test]
    fn test_recent() {
        let log = make_log();
        let recent = log.recent(2);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].action, AuditAction::PasswordChange);
    }

    #[test]
    fn test_search() {
        let log = make_log();
        let results = log.search("password");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_search_by_action_label() {
        let log = make_log();
        let results = log.search("transfer");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_latest() {
        let log = make_log();
        let latest = log.latest().unwrap();
        assert_eq!(latest.action, AuditAction::PasswordChange);
    }

    #[test]
    fn test_get_by_index() {
        let log = make_log();
        let entry = log.get(2).unwrap();
        assert_eq!(entry.action, AuditAction::SpendingPolicyChange);
    }

    #[test]
    fn test_severity_ordering() {
        assert!(Severity::Info < Severity::Warning);
        assert!(Severity::Warning < Severity::Critical);
    }

    #[test]
    fn test_to_csv() {
        let log = make_log();
        let csv = log.to_csv();
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines.len(), 5); // header + 4 entries
        assert!(lines[0].starts_with("index,timestamp"));
    }

    #[test]
    fn test_metadata() {
        let mut log = AuditLog::new();
        let mut meta = std::collections::HashMap::new();
        meta.insert("tx_hash".into(), "0xabc".into());
        meta.insert("amount".into(), "1000".into());
        log.append(
            AuditAction::TransferSend,
            Severity::Info,
            "alice",
            "Transfer",
            meta,
        );
        let entry = log.latest().unwrap();
        assert_eq!(entry.metadata.get("tx_hash").unwrap(), "0xabc");
        assert!(entry.verify());
    }

    #[test]
    fn test_custom_action() {
        let mut log = AuditLog::new();
        log.info(
            AuditAction::Custom("import_csv".into()),
            "admin",
            "Imported CSV data",
        );
        assert_eq!(log.latest().unwrap().action.label(), "custom.import_csv");
    }

    #[test]
    fn test_max_entries() {
        let mut log = AuditLog::with_max_entries(3);
        for i in 0..5 {
            log.info(AuditAction::TransferSend, "alice", &format!("tx {}", i));
        }
        assert_eq!(log.len(), 3);
        // Oldest entries evicted
        assert_eq!(log.entries[0].index, 2);
    }

    #[test]
    fn test_persistence_roundtrip() {
        let dir = std::env::temp_dir().join("evap_audit_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("audit.json");

        let log = make_log();
        log.save(&path).unwrap();

        let loaded = AuditLog::load(&path).unwrap();
        assert_eq!(loaded.len(), 4);
        assert!(loaded.verify_chain().is_ok());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_empty_log_verify() {
        let log = AuditLog::new();
        assert!(log.verify_chain().is_ok());
    }

    #[test]
    fn test_unique_hashes() {
        let log = make_log();
        let hashes: Vec<&String> = log.entries.iter().map(|e| &e.hash).collect();
        for i in 0..hashes.len() {
            for j in (i + 1)..hashes.len() {
                assert_ne!(hashes[i], hashes[j], "duplicate hash at {} and {}", i, j);
            }
        }
    }

    #[test]
    fn test_default_trait() {
        let log = AuditLog::default();
        assert!(log.is_empty());
    }
}
