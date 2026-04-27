// wallet/src/migrate.rs — Import wallets from external formats
//
// Supports importing from:
//   - Mnemonic phrases (BIP-39 compatible, re-derived as ML-DSA)
//   - Raw private key hex
//   - MetaMask JSON keystore (v3)
//   - Phantom export JSON
//   - Generic JSON keystore
//
// Also supports exporting EvaporChain wallet data for backup/migration.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MigrateError {
    #[error("unsupported format: {0}")]
    UnsupportedFormat(String),
    #[error("invalid keystore: {0}")]
    InvalidKeystore(String),
    #[error("decryption failed: {0}")]
    DecryptionFailed(String),
    #[error("invalid mnemonic: {0}")]
    InvalidMnemonic(String),
    #[error("invalid private key: {0}")]
    InvalidKey(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("json error: {0}")]
    Json(String),
    #[error("migration validation failed: {0}")]
    ValidationFailed(String),
}

// ── Source format detection ───────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceFormat {
    Mnemonic,
    RawKey,
    MetaMaskV3,
    PhantomJson,
    GenericKeystore,
    EvaporChainBackup,
}

impl SourceFormat {
    pub fn name(&self) -> &'static str {
        match self {
            SourceFormat::Mnemonic => "BIP-39 Mnemonic",
            SourceFormat::RawKey => "Raw Private Key",
            SourceFormat::MetaMaskV3 => "MetaMask V3 Keystore",
            SourceFormat::PhantomJson => "Phantom Export",
            SourceFormat::GenericKeystore => "Generic JSON Keystore",
            SourceFormat::EvaporChainBackup => "EvaporChain Backup",
        }
    }
}

/// Detect the source format from file content or string
pub fn detect_format(input: &str) -> Option<SourceFormat> {
    let trimmed = input.trim();

    // Mnemonic: 12 or 24 space-separated words
    let word_count = trimmed.split_whitespace().count();
    if word_count == 12 || word_count == 24 {
        let all_alpha = trimmed
            .split_whitespace()
            .all(|w| w.chars().all(|c| c.is_ascii_alphabetic()));
        if all_alpha {
            return Some(SourceFormat::Mnemonic);
        }
    }

    // Raw hex key: 64 hex chars (32 bytes) or 128 hex chars (64 bytes)
    if (trimmed.len() == 64 || trimmed.len() == 128)
        && trimmed.chars().all(|c| c.is_ascii_hexdigit())
    {
        return Some(SourceFormat::RawKey);
    }

    // 0x-prefixed hex
    if let Some(hex) = trimmed.strip_prefix("0x") {
        if (hex.len() == 64 || hex.len() == 128)
            && hex.chars().all(|c| c.is_ascii_hexdigit())
        {
            return Some(SourceFormat::RawKey);
        }
    }

    // JSON-based formats
    if trimmed.starts_with('{') {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
            // MetaMask V3: has "version": 3 and "crypto" key
            if v.get("version").and_then(|v| v.as_i64()) == Some(3)
                && (v.get("crypto").is_some() || v.get("Crypto").is_some())
            {
                return Some(SourceFormat::MetaMaskV3);
            }
            // EvaporChain backup: has "evaporchain_version"
            if v.get("evaporchain_version").is_some() {
                return Some(SourceFormat::EvaporChainBackup);
            }
            // Phantom: has "mnemonic" or "secretKey" field
            if v.get("mnemonic").is_some() || v.get("secretKey").is_some() {
                return Some(SourceFormat::PhantomJson);
            }
            // Generic keystore
            if v.get("address").is_some() || v.get("crypto").is_some() {
                return Some(SourceFormat::GenericKeystore);
            }
        }
    }

    None
}

// ── Import result ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedAccount {
    pub label: String,
    pub source_format: String,
    pub original_address: Option<String>,
    pub note: String,
    pub imported_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationPlan {
    pub source: String,
    pub format: String,
    pub accounts_found: usize,
    pub accounts: Vec<ImportedAccount>,
    pub warnings: Vec<String>,
}

impl MigrationPlan {
    pub fn new(source: &str, format: SourceFormat) -> Self {
        Self {
            source: source.to_string(),
            format: format.name().to_string(),
            accounts_found: 0,
            accounts: Vec::new(),
            warnings: Vec::new(),
        }
    }

    pub fn add_account(&mut self, account: ImportedAccount) {
        self.accounts_found += 1;
        self.accounts.push(account);
    }

    pub fn add_warning(&mut self, msg: &str) {
        self.warnings.push(msg.to_string());
    }
}

// ── Migrator ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationRecord {
    pub id: String,
    pub source: String,
    pub format: String,
    pub accounts_imported: usize,
    pub timestamp: String,
    pub status: MigrationStatus,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MigrationStatus {
    Planned,
    InProgress,
    Completed,
    Failed,
    RolledBack,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Migrator {
    pub history: Vec<MigrationRecord>,
    pub supported_formats: Vec<String>,
}

impl Migrator {
    pub fn new() -> Self {
        Self {
            history: Vec::new(),
            supported_formats: vec![
                "BIP-39 Mnemonic".to_string(),
                "Raw Private Key".to_string(),
                "MetaMask V3 Keystore".to_string(),
                "Phantom Export".to_string(),
                "Generic JSON Keystore".to_string(),
                "EvaporChain Backup".to_string(),
            ],
        }
    }

    /// Plan a migration from raw input (dry run)
    pub fn plan_from_input(&self, input: &str) -> Result<MigrationPlan, MigrateError> {
        let format = detect_format(input)
            .ok_or_else(|| MigrateError::UnsupportedFormat("could not detect format".into()))?;

        let mut plan = MigrationPlan::new("direct_input", format);

        match format {
            SourceFormat::Mnemonic => {
                let words: Vec<&str> = input.split_whitespace().collect();
                plan.add_account(ImportedAccount {
                    label: "imported-mnemonic".to_string(),
                    source_format: format.name().to_string(),
                    original_address: None,
                    note: format!("{}-word mnemonic phrase", words.len()),
                    imported_at: chrono::Utc::now().to_rfc3339(),
                });
                plan.add_warning(
                    "Mnemonic will be re-derived as ML-DSA key (not the original secp256k1 key)",
                );
            }
            SourceFormat::RawKey => {
                plan.add_account(ImportedAccount {
                    label: "imported-key".to_string(),
                    source_format: format.name().to_string(),
                    original_address: None,
                    note: "Raw private key import".to_string(),
                    imported_at: chrono::Utc::now().to_rfc3339(),
                });
                plan.add_warning("Key will be wrapped in ML-DSA envelope");
            }
            SourceFormat::MetaMaskV3 => {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(input) {
                    let addr = v
                        .get("address")
                        .and_then(|a| a.as_str())
                        .map(|s| format!("0x{}", s));
                    plan.add_account(ImportedAccount {
                        label: "imported-metamask".to_string(),
                        source_format: format.name().to_string(),
                        original_address: addr,
                        note: "MetaMask V3 keystore".to_string(),
                        imported_at: chrono::Utc::now().to_rfc3339(),
                    });
                    plan.add_warning("MetaMask key (secp256k1) cannot be used directly; a new ML-DSA key will be generated");
                }
            }
            SourceFormat::PhantomJson => {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(input) {
                    let has_mnemonic = v.get("mnemonic").is_some();
                    plan.add_account(ImportedAccount {
                        label: "imported-phantom".to_string(),
                        source_format: format.name().to_string(),
                        original_address: v
                            .get("publicKey")
                            .and_then(|p| p.as_str())
                            .map(String::from),
                        note: if has_mnemonic {
                            "Phantom mnemonic export".to_string()
                        } else {
                            "Phantom secret key export".to_string()
                        },
                        imported_at: chrono::Utc::now().to_rfc3339(),
                    });
                    plan.add_warning("Solana Ed25519 key will be re-derived as ML-DSA");
                }
            }
            SourceFormat::GenericKeystore => {
                plan.add_account(ImportedAccount {
                    label: "imported-keystore".to_string(),
                    source_format: format.name().to_string(),
                    original_address: None,
                    note: "Generic JSON keystore".to_string(),
                    imported_at: chrono::Utc::now().to_rfc3339(),
                });
            }
            SourceFormat::EvaporChainBackup => {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(input) {
                    let count = v
                        .get("accounts")
                        .and_then(|a| a.as_array())
                        .map(|a| a.len())
                        .unwrap_or(1);
                    for i in 0..count {
                        plan.add_account(ImportedAccount {
                            label: format!("restored-{}", i),
                            source_format: format.name().to_string(),
                            original_address: None,
                            note: "EvaporChain backup restore".to_string(),
                            imported_at: chrono::Utc::now().to_rfc3339(),
                        });
                    }
                }
            }
        }

        Ok(plan)
    }

    /// Plan a migration from a file path
    pub fn plan_from_file(&self, path: &Path) -> Result<MigrationPlan, MigrateError> {
        let content =
            std::fs::read_to_string(path).map_err(|e| MigrateError::Io(e.to_string()))?;
        let mut plan = self.plan_from_input(&content)?;
        plan.source = path.display().to_string();
        Ok(plan)
    }

    /// Record a completed migration
    pub fn record_migration(
        &mut self,
        plan: &MigrationPlan,
        status: MigrationStatus,
        notes: Vec<String>,
    ) -> String {
        let id = format!("mig-{}", self.history.len() + 1);
        self.history.push(MigrationRecord {
            id: id.clone(),
            source: plan.source.clone(),
            format: plan.format.clone(),
            accounts_imported: plan.accounts_found,
            timestamp: chrono::Utc::now().to_rfc3339(),
            status,
            notes,
        });
        id
    }

    /// List migration history
    pub fn list_history(&self) -> &[MigrationRecord] {
        &self.history
    }

    /// Get a specific migration by ID
    pub fn get_migration(&self, id: &str) -> Option<&MigrationRecord> {
        self.history.iter().find(|m| m.id == id)
    }

    /// Rollback a migration (mark as rolled back)
    pub fn rollback(&mut self, id: &str) -> Result<(), MigrateError> {
        if let Some(m) = self.history.iter_mut().find(|m| m.id == id) {
            if m.status != MigrationStatus::Completed {
                return Err(MigrateError::ValidationFailed(format!(
                    "can only rollback completed migrations, current status: {:?}",
                    m.status
                )));
            }
            m.status = MigrationStatus::RolledBack;
            Ok(())
        } else {
            Err(MigrateError::ValidationFailed(format!(
                "migration not found: {}",
                id
            )))
        }
    }

    /// Validate a mnemonic phrase (basic word count + alpha check)
    pub fn validate_mnemonic(phrase: &str) -> Result<usize, MigrateError> {
        let words: Vec<&str> = phrase.split_whitespace().collect();
        if words.len() != 12 && words.len() != 24 {
            return Err(MigrateError::InvalidMnemonic(format!(
                "expected 12 or 24 words, got {}",
                words.len()
            )));
        }
        for (i, w) in words.iter().enumerate() {
            if !w.chars().all(|c| c.is_ascii_alphabetic()) {
                return Err(MigrateError::InvalidMnemonic(format!(
                    "word {} ('{}') contains non-alphabetic characters",
                    i + 1,
                    w
                )));
            }
        }
        Ok(words.len())
    }

    /// Validate a hex private key
    pub fn validate_private_key(hex_key: &str) -> Result<usize, MigrateError> {
        let key = hex_key.trim().strip_prefix("0x").unwrap_or(hex_key.trim());
        if key.len() != 64 && key.len() != 128 {
            return Err(MigrateError::InvalidKey(format!(
                "expected 64 or 128 hex chars, got {}",
                key.len()
            )));
        }
        if !key.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(MigrateError::InvalidKey(
                "contains non-hex characters".into(),
            ));
        }
        Ok(key.len() / 2)
    }

    /// Export format compatibility info
    pub fn compatibility_matrix() -> HashMap<String, Vec<String>> {
        let mut m = HashMap::new();
        m.insert(
            "MetaMask".to_string(),
            vec![
                "V3 JSON keystore import".to_string(),
                "Mnemonic phrase import".to_string(),
                "Note: secp256k1 keys re-derived as ML-DSA".to_string(),
            ],
        );
        m.insert(
            "Phantom".to_string(),
            vec![
                "JSON export import".to_string(),
                "Mnemonic phrase import".to_string(),
                "Note: Ed25519 keys re-derived as ML-DSA".to_string(),
            ],
        );
        m.insert(
            "Trust Wallet".to_string(),
            vec![
                "Mnemonic phrase import".to_string(),
                "Note: multi-chain keys re-derived as ML-DSA".to_string(),
            ],
        );
        m.insert(
            "Ledger/Trezor".to_string(),
            vec![
                "Mnemonic backup import".to_string(),
                "Note: hardware keys stay on device — only mnemonic can migrate".to_string(),
            ],
        );
        m
    }

    /// JSON persistence
    pub fn save(&self, path: &Path) -> Result<(), MigrateError> {
        let json = serde_json::to_string_pretty(self).map_err(|e| MigrateError::Json(e.to_string()))?;
        std::fs::write(path, json).map_err(|e| MigrateError::Io(e.to_string()))
    }

    pub fn load(path: &Path) -> Result<Self, MigrateError> {
        let data = std::fs::read_to_string(path).map_err(|e| MigrateError::Io(e.to_string()))?;
        serde_json::from_str(&data).map_err(|e| MigrateError::Json(e.to_string()))
    }
}

// ── Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_mnemonic_12() {
        let input = "abandon ability able about above absent absorb abstract absurd abuse access accident";
        assert_eq!(detect_format(input), Some(SourceFormat::Mnemonic));
    }

    #[test]
    fn test_detect_mnemonic_24() {
        let input = "abandon ability able about above absent absorb abstract absurd abuse access accident abandon ability able about above absent absorb abstract absurd abuse access accident";
        assert_eq!(detect_format(input), Some(SourceFormat::Mnemonic));
    }

    #[test]
    fn test_detect_raw_key_64() {
        let key = "a".repeat(64);
        assert_eq!(detect_format(&key), Some(SourceFormat::RawKey));
    }

    #[test]
    fn test_detect_raw_key_0x_prefix() {
        let key = format!("0x{}", "b".repeat(64));
        assert_eq!(detect_format(&key), Some(SourceFormat::RawKey));
    }

    #[test]
    fn test_detect_metamask_v3() {
        let json = r#"{"version":3,"crypto":{"cipher":"aes-128-ctr"},"address":"abc123"}"#;
        assert_eq!(detect_format(json), Some(SourceFormat::MetaMaskV3));
    }

    #[test]
    fn test_detect_phantom() {
        let json = r#"{"mnemonic":"test words","publicKey":"abc"}"#;
        assert_eq!(detect_format(json), Some(SourceFormat::PhantomJson));
    }

    #[test]
    fn test_detect_evaporchain_backup() {
        let json = r#"{"evaporchain_version":"1.0","accounts":[]}"#;
        assert_eq!(detect_format(json), Some(SourceFormat::EvaporChainBackup));
    }

    #[test]
    fn test_detect_generic_keystore() {
        let json = r#"{"address":"0xabc","crypto":{}}"#;
        // version != 3, so not MetaMask, but has crypto → generic
        assert_eq!(detect_format(json), Some(SourceFormat::GenericKeystore));
    }

    #[test]
    fn test_detect_unknown() {
        assert_eq!(detect_format("hello world"), None);
        assert_eq!(detect_format("12345"), None);
    }

    #[test]
    fn test_plan_mnemonic() {
        let m = Migrator::new();
        let input = "abandon ability able about above absent absorb abstract absurd abuse access accident";
        let plan = m.plan_from_input(input).unwrap();
        assert_eq!(plan.format, "BIP-39 Mnemonic");
        assert_eq!(plan.accounts_found, 1);
        assert!(!plan.warnings.is_empty());
    }

    #[test]
    fn test_plan_raw_key() {
        let m = Migrator::new();
        let key = "a".repeat(64);
        let plan = m.plan_from_input(&key).unwrap();
        assert_eq!(plan.format, "Raw Private Key");
        assert_eq!(plan.accounts_found, 1);
    }

    #[test]
    fn test_plan_metamask() {
        let m = Migrator::new();
        let json = r#"{"version":3,"crypto":{"cipher":"aes-128-ctr"},"address":"deadbeef"}"#;
        let plan = m.plan_from_input(json).unwrap();
        assert_eq!(plan.format, "MetaMask V3 Keystore");
        assert_eq!(plan.accounts[0].original_address, Some("0xdeadbeef".to_string()));
    }

    #[test]
    fn test_plan_evaporchain_multi_account() {
        let m = Migrator::new();
        let json = r#"{"evaporchain_version":"1.0","accounts":[{},{},{}]}"#;
        let plan = m.plan_from_input(json).unwrap();
        assert_eq!(plan.accounts_found, 3);
    }

    #[test]
    fn test_plan_unknown_format() {
        let m = Migrator::new();
        assert!(m.plan_from_input("not a valid format").is_err());
    }

    #[test]
    fn test_record_and_list_history() {
        let mut m = Migrator::new();
        let plan = MigrationPlan::new("test", SourceFormat::Mnemonic);
        let id = m.record_migration(&plan, MigrationStatus::Completed, vec!["ok".into()]);
        assert_eq!(m.list_history().len(), 1);
        let rec = m.get_migration(&id).unwrap();
        assert_eq!(rec.status, MigrationStatus::Completed);
    }

    #[test]
    fn test_rollback() {
        let mut m = Migrator::new();
        let plan = MigrationPlan::new("test", SourceFormat::RawKey);
        let id = m.record_migration(&plan, MigrationStatus::Completed, vec![]);
        m.rollback(&id).unwrap();
        assert_eq!(
            m.get_migration(&id).unwrap().status,
            MigrationStatus::RolledBack
        );
    }

    #[test]
    fn test_rollback_non_completed_fails() {
        let mut m = Migrator::new();
        let plan = MigrationPlan::new("test", SourceFormat::RawKey);
        let id = m.record_migration(&plan, MigrationStatus::Failed, vec![]);
        assert!(m.rollback(&id).is_err());
    }

    #[test]
    fn test_rollback_not_found() {
        let mut m = Migrator::new();
        assert!(m.rollback("mig-999").is_err());
    }

    #[test]
    fn test_validate_mnemonic_12_words() {
        let phrase = "abandon ability able about above absent absorb abstract absurd abuse access accident";
        assert_eq!(Migrator::validate_mnemonic(phrase).unwrap(), 12);
    }

    #[test]
    fn test_validate_mnemonic_bad_count() {
        assert!(Migrator::validate_mnemonic("one two three").is_err());
    }

    #[test]
    fn test_validate_mnemonic_non_alpha() {
        let phrase = "abandon ability able about above absent absorb abstract absurd abuse access 123abc";
        assert!(Migrator::validate_mnemonic(phrase).is_err());
    }

    #[test]
    fn test_validate_private_key() {
        let key = "a".repeat(64);
        assert_eq!(Migrator::validate_private_key(&key).unwrap(), 32);
    }

    #[test]
    fn test_validate_private_key_0x() {
        let key = format!("0x{}", "b".repeat(64));
        assert_eq!(Migrator::validate_private_key(&key).unwrap(), 32);
    }

    #[test]
    fn test_validate_private_key_bad_length() {
        assert!(Migrator::validate_private_key("abcd").is_err());
    }

    #[test]
    fn test_validate_private_key_non_hex() {
        let key = format!("{}zz", "a".repeat(62));
        assert!(Migrator::validate_private_key(&key).is_err());
    }

    #[test]
    fn test_compatibility_matrix() {
        let matrix = Migrator::compatibility_matrix();
        assert!(matrix.contains_key("MetaMask"));
        assert!(matrix.contains_key("Phantom"));
        assert!(matrix.contains_key("Trust Wallet"));
        assert!(matrix.contains_key("Ledger/Trezor"));
    }

    #[test]
    fn test_save_load() {
        let path = std::env::temp_dir().join(format!("evap_test_migrator_{}.json", std::process::id()));
        let mut m = Migrator::new();
        let plan = MigrationPlan::new("test", SourceFormat::Mnemonic);
        m.record_migration(&plan, MigrationStatus::Completed, vec!["note".into()]);
        m.save(&path).unwrap();
        let loaded = Migrator::load(&path).unwrap();
        assert_eq!(loaded.history.len(), 1);
        assert_eq!(loaded.history[0].notes, vec!["note".to_string()]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_source_format_names() {
        assert_eq!(SourceFormat::Mnemonic.name(), "BIP-39 Mnemonic");
        assert_eq!(SourceFormat::MetaMaskV3.name(), "MetaMask V3 Keystore");
        assert_eq!(SourceFormat::PhantomJson.name(), "Phantom Export");
    }
}
