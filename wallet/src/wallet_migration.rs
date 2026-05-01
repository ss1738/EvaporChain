//! Wallet import and migration system.
//!
//! Supports importing accounts from external wallets (MetaMask, Phantom,
//! Trust Wallet, Ledger, Trezor, Exodus, and custom sources). Tracks
//! migration jobs through their lifecycle, validates key formats, and
//! generates migration reports. Persists state as JSON.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum MigrationError {
    #[error("job not found: {0}")]
    JobNotFound(String),
    #[error("invalid status transition: {0}")]
    InvalidStatus(String),
    #[error("validation error: {0}")]
    Validation(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

// ──────────────────────────── Enums ──────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SourceWallet {
    MetaMask,
    Phantom,
    TrustWallet,
    Ledger,
    Trezor,
    Exodus,
    Custom(String),
}

impl std::fmt::Display for SourceWallet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MetaMask => write!(f, "MetaMask"),
            Self::Phantom => write!(f, "Phantom"),
            Self::TrustWallet => write!(f, "TrustWallet"),
            Self::Ledger => write!(f, "Ledger"),
            Self::Trezor => write!(f, "Trezor"),
            Self::Exodus => write!(f, "Exodus"),
            Self::Custom(name) => write!(f, "Custom({})", name),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MigrationStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyFormat {
    Hex,
    Base58,
    Bech32,
    Mnemonic12,
    Mnemonic24,
}

// ──────────────────────────── Structs ────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationJob {
    pub id: String,
    pub source: SourceWallet,
    pub status: MigrationStatus,
    pub created_at: String,
    pub completed_at: Option<String>,
    pub accounts_found: u32,
    pub accounts_imported: u32,
    pub tokens_found: u32,
    pub tokens_imported: u32,
    pub nfts_found: u32,
    pub nfts_imported: u32,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedAccount {
    pub original_address: String,
    pub new_address: String,
    pub source: SourceWallet,
    pub key_format: KeyFormat,
    pub label: Option<String>,
    pub imported_at: String,
    pub balance_snapshot: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationReport {
    pub job_id: String,
    pub source: SourceWallet,
    pub duration_secs: u64,
    pub accounts_imported: u32,
    pub tokens_imported: u32,
    pub nfts_imported: u32,
    pub errors: Vec<String>,
    pub success: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationStats {
    pub total_migrations: usize,
    pub completed: usize,
    pub failed: usize,
    pub total_accounts_imported: u32,
    pub total_tokens_imported: u32,
    pub sources_used: Vec<String>,
}

// ──────────────────────────── Main Store ─────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WalletMigrator {
    pub jobs: HashMap<String, MigrationJob>,
    pub imported_accounts: Vec<ImportedAccount>,
    pub reports: Vec<MigrationReport>,
}

impl WalletMigrator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new migration job and return its ID.
    pub fn start_migration(&mut self, source: SourceWallet) -> String {
        let id = uuid_v4();
        let job = MigrationJob {
            id: id.clone(),
            source,
            status: MigrationStatus::Pending,
            created_at: chrono::Utc::now().to_rfc3339(),
            completed_at: None,
            accounts_found: 0,
            accounts_imported: 0,
            tokens_found: 0,
            tokens_imported: 0,
            nfts_found: 0,
            nfts_imported: 0,
            errors: Vec::new(),
            warnings: Vec::new(),
        };
        self.jobs.insert(id.clone(), job);
        id
    }

    /// Set the number of discovered accounts for a job.
    pub fn discover_accounts(&mut self, job_id: &str, count: u32) -> Result<(), MigrationError> {
        let job = self
            .jobs
            .get_mut(job_id)
            .ok_or_else(|| MigrationError::JobNotFound(job_id.to_string()))?;
        if job.status == MigrationStatus::Completed
            || job.status == MigrationStatus::Failed
            || job.status == MigrationStatus::Cancelled
        {
            return Err(MigrationError::InvalidStatus(format!(
                "cannot discover on {:?} job",
                job.status
            )));
        }
        job.status = MigrationStatus::InProgress;
        job.accounts_found = count;
        Ok(())
    }

    /// Import an account into a running migration job.
    pub fn import_account(
        &mut self,
        job_id: &str,
        account: ImportedAccount,
    ) -> Result<(), MigrationError> {
        let job = self
            .jobs
            .get_mut(job_id)
            .ok_or_else(|| MigrationError::JobNotFound(job_id.to_string()))?;
        if job.status != MigrationStatus::InProgress && job.status != MigrationStatus::Pending {
            return Err(MigrationError::InvalidStatus(format!(
                "cannot import on {:?} job",
                job.status
            )));
        }
        job.status = MigrationStatus::InProgress;
        job.accounts_imported += 1;
        self.imported_accounts.push(account);
        Ok(())
    }

    /// Complete a migration and generate a report.
    pub fn complete_migration(&mut self, job_id: &str) -> Result<MigrationReport, MigrationError> {
        let job = self
            .jobs
            .get_mut(job_id)
            .ok_or_else(|| MigrationError::JobNotFound(job_id.to_string()))?;
        if job.status == MigrationStatus::Completed
            || job.status == MigrationStatus::Failed
            || job.status == MigrationStatus::Cancelled
        {
            return Err(MigrationError::InvalidStatus(format!(
                "cannot complete {:?} job",
                job.status
            )));
        }
        let now = chrono::Utc::now().to_rfc3339();
        job.status = MigrationStatus::Completed;
        job.completed_at = Some(now.clone());

        // Calculate rough duration
        let duration = chrono::DateTime::parse_from_rfc3339(&now)
            .ok()
            .and_then(|end| {
                chrono::DateTime::parse_from_rfc3339(&job.created_at)
                    .ok()
                    .map(|start| (end - start).num_seconds().max(0) as u64)
            })
            .unwrap_or(0);

        let report = MigrationReport {
            job_id: job_id.to_string(),
            source: job.source.clone(),
            duration_secs: duration,
            accounts_imported: job.accounts_imported,
            tokens_imported: job.tokens_imported,
            nfts_imported: job.nfts_imported,
            errors: job.errors.clone(),
            success: job.errors.is_empty(),
        };
        self.reports.push(report.clone());
        Ok(report)
    }

    /// Mark a migration as failed.
    pub fn fail_migration(&mut self, job_id: &str, reason: &str) -> Result<(), MigrationError> {
        let job = self
            .jobs
            .get_mut(job_id)
            .ok_or_else(|| MigrationError::JobNotFound(job_id.to_string()))?;
        if job.status == MigrationStatus::Completed || job.status == MigrationStatus::Cancelled {
            return Err(MigrationError::InvalidStatus(format!(
                "cannot fail {:?} job",
                job.status
            )));
        }
        job.status = MigrationStatus::Failed;
        job.completed_at = Some(chrono::Utc::now().to_rfc3339());
        job.errors.push(reason.to_string());
        Ok(())
    }

    /// Cancel a migration.
    pub fn cancel_migration(&mut self, job_id: &str) -> Result<(), MigrationError> {
        let job = self
            .jobs
            .get_mut(job_id)
            .ok_or_else(|| MigrationError::JobNotFound(job_id.to_string()))?;
        if job.status == MigrationStatus::Completed || job.status == MigrationStatus::Failed {
            return Err(MigrationError::InvalidStatus(format!(
                "cannot cancel {:?} job",
                job.status
            )));
        }
        job.status = MigrationStatus::Cancelled;
        job.completed_at = Some(chrono::Utc::now().to_rfc3339());
        Ok(())
    }

    /// Look up a job by ID.
    pub fn get_job(&self, id: &str) -> Option<&MigrationJob> {
        self.jobs.get(id)
    }

    /// Return all jobs that are Pending or InProgress.
    pub fn active_migrations(&self) -> Vec<&MigrationJob> {
        self.jobs
            .values()
            .filter(|j| {
                j.status == MigrationStatus::Pending || j.status == MigrationStatus::InProgress
            })
            .collect()
    }

    /// Return all imported accounts from a given source.
    pub fn imported_from(&self, source: &SourceWallet) -> Vec<&ImportedAccount> {
        self.imported_accounts
            .iter()
            .filter(|a| &a.source == source)
            .collect()
    }

    /// Find an imported account by its original address.
    pub fn find_by_original_address(&self, addr: &str) -> Option<&ImportedAccount> {
        self.imported_accounts
            .iter()
            .find(|a| a.original_address == addr)
    }

    /// Basic key format validation.
    pub fn validate_key_format(&self, key: &str, format: &KeyFormat) -> bool {
        match format {
            KeyFormat::Hex => {
                !key.is_empty()
                    && key.len().is_multiple_of(2)
                    && key.chars().all(|c| c.is_ascii_hexdigit())
            }
            KeyFormat::Base58 => {
                const BASE58_CHARS: &str =
                    "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
                !key.is_empty() && key.chars().all(|c| BASE58_CHARS.contains(c))
            }
            KeyFormat::Bech32 => !key.is_empty() && key.contains('1') && key.len() >= 8,
            KeyFormat::Mnemonic12 => {
                let words: Vec<&str> = key.split_whitespace().collect();
                words.len() == 12
            }
            KeyFormat::Mnemonic24 => {
                let words: Vec<&str> = key.split_whitespace().collect();
                words.len() == 24
            }
        }
    }

    /// List all supported wallet sources.
    pub fn supported_sources() -> Vec<SourceWallet> {
        vec![
            SourceWallet::MetaMask,
            SourceWallet::Phantom,
            SourceWallet::TrustWallet,
            SourceWallet::Ledger,
            SourceWallet::Trezor,
            SourceWallet::Exodus,
        ]
    }

    /// Compute aggregate statistics.
    pub fn stats(&self) -> MigrationStats {
        let completed = self
            .jobs
            .values()
            .filter(|j| j.status == MigrationStatus::Completed)
            .count();
        let failed = self
            .jobs
            .values()
            .filter(|j| j.status == MigrationStatus::Failed)
            .count();
        let total_accounts_imported: u32 = self.jobs.values().map(|j| j.accounts_imported).sum();
        let total_tokens_imported: u32 = self.jobs.values().map(|j| j.tokens_imported).sum();
        let mut sources_used: Vec<String> = self
            .jobs
            .values()
            .map(|j| j.source.to_string())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        sources_used.sort();

        MigrationStats {
            total_migrations: self.jobs.len(),
            completed,
            failed,
            total_accounts_imported,
            total_tokens_imported,
            sources_used,
        }
    }

    // -- persistence ----------------------------------------------------------

    pub fn load(path: &Path) -> Result<Self, MigrationError> {
        let data = std::fs::read_to_string(path)?;
        let migrator: Self = serde_json::from_str(&data)?;
        Ok(migrator)
    }

    pub fn save(&self, path: &Path) -> Result<(), MigrationError> {
        let data = serde_json::to_string_pretty(self)?;
        std::fs::write(path, data)?;
        Ok(())
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ──────────────────────────── Helpers ────────────────────────────────

fn uuid_v4() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        d.as_nanos() as u32,
        (d.as_nanos() >> 32) as u16,
        ((d.as_nanos() >> 48) as u16 & 0x0fff) | 0x4000,
        ((d.as_nanos() >> 60) as u16 & 0x3fff) | 0x8000,
        (d.as_nanos() >> 64) as u64 ^ std::process::id() as u64,
    )
}

// ──────────────────────────── Tests ─────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::temp_dir;
    use std::process::id;

    fn temp_path(name: &str) -> std::path::PathBuf {
        temp_dir().join(format!("wallet_migration_test_{}_{}", id(), name))
    }

    fn sample_account(source: SourceWallet) -> ImportedAccount {
        ImportedAccount {
            original_address: "0xabc123".to_string(),
            new_address: "evap1xyz789".to_string(),
            source,
            key_format: KeyFormat::Hex,
            label: Some("Main".to_string()),
            imported_at: chrono::Utc::now().to_rfc3339(),
            balance_snapshot: 1000,
        }
    }

    #[test]
    fn test_new_migrator_is_empty() {
        let m = WalletMigrator::new();
        assert!(m.jobs.is_empty());
        assert!(m.imported_accounts.is_empty());
        assert!(m.reports.is_empty());
    }

    #[test]
    fn test_start_migration_creates_job() {
        let mut m = WalletMigrator::new();
        let id = m.start_migration(SourceWallet::MetaMask);
        assert!(!id.is_empty());
        assert_eq!(m.jobs.len(), 1);
        let job = m.get_job(&id).unwrap();
        assert_eq!(job.status, MigrationStatus::Pending);
        assert_eq!(job.source, SourceWallet::MetaMask);
    }

    #[test]
    fn test_discover_accounts() {
        let mut m = WalletMigrator::new();
        let id = m.start_migration(SourceWallet::Phantom);
        m.discover_accounts(&id, 5).unwrap();
        let job = m.get_job(&id).unwrap();
        assert_eq!(job.accounts_found, 5);
        assert_eq!(job.status, MigrationStatus::InProgress);
    }

    #[test]
    fn test_discover_accounts_job_not_found() {
        let mut m = WalletMigrator::new();
        let result = m.discover_accounts("nonexistent", 5);
        assert!(result.is_err());
    }

    #[test]
    fn test_import_account() {
        let mut m = WalletMigrator::new();
        let id = m.start_migration(SourceWallet::Ledger);
        let acct = sample_account(SourceWallet::Ledger);
        m.import_account(&id, acct).unwrap();
        assert_eq!(m.imported_accounts.len(), 1);
        let job = m.get_job(&id).unwrap();
        assert_eq!(job.accounts_imported, 1);
    }

    #[test]
    fn test_import_account_job_not_found() {
        let mut m = WalletMigrator::new();
        let acct = sample_account(SourceWallet::MetaMask);
        let result = m.import_account("bad_id", acct);
        assert!(result.is_err());
    }

    #[test]
    fn test_complete_migration() {
        let mut m = WalletMigrator::new();
        let id = m.start_migration(SourceWallet::Exodus);
        m.discover_accounts(&id, 2).unwrap();
        let acct = sample_account(SourceWallet::Exodus);
        m.import_account(&id, acct).unwrap();
        let report = m.complete_migration(&id).unwrap();
        assert!(report.success);
        assert_eq!(report.accounts_imported, 1);
        let job = m.get_job(&id).unwrap();
        assert_eq!(job.status, MigrationStatus::Completed);
        assert!(job.completed_at.is_some());
    }

    #[test]
    fn test_complete_already_completed() {
        let mut m = WalletMigrator::new();
        let id = m.start_migration(SourceWallet::MetaMask);
        m.complete_migration(&id).unwrap();
        let result = m.complete_migration(&id);
        assert!(result.is_err());
    }

    #[test]
    fn test_fail_migration() {
        let mut m = WalletMigrator::new();
        let id = m.start_migration(SourceWallet::Trezor);
        m.fail_migration(&id, "connection lost").unwrap();
        let job = m.get_job(&id).unwrap();
        assert_eq!(job.status, MigrationStatus::Failed);
        assert_eq!(job.errors.len(), 1);
        assert!(job.completed_at.is_some());
    }

    #[test]
    fn test_cancel_migration() {
        let mut m = WalletMigrator::new();
        let id = m.start_migration(SourceWallet::Phantom);
        m.cancel_migration(&id).unwrap();
        let job = m.get_job(&id).unwrap();
        assert_eq!(job.status, MigrationStatus::Cancelled);
    }

    #[test]
    fn test_cancel_completed_fails() {
        let mut m = WalletMigrator::new();
        let id = m.start_migration(SourceWallet::MetaMask);
        m.complete_migration(&id).unwrap();
        let result = m.cancel_migration(&id);
        assert!(result.is_err());
    }

    #[test]
    fn test_active_migrations() {
        let mut m = WalletMigrator::new();
        let id1 = m.start_migration(SourceWallet::MetaMask);
        let id2 = m.start_migration(SourceWallet::Phantom);
        m.complete_migration(&id1).unwrap();
        let active = m.active_migrations();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, id2);
    }

    #[test]
    fn test_imported_from() {
        let mut m = WalletMigrator::new();
        let id = m.start_migration(SourceWallet::MetaMask);
        let acct = sample_account(SourceWallet::MetaMask);
        m.import_account(&id, acct).unwrap();
        let id2 = m.start_migration(SourceWallet::Phantom);
        let acct2 = ImportedAccount {
            original_address: "phantom_addr".to_string(),
            new_address: "evap_new".to_string(),
            source: SourceWallet::Phantom,
            key_format: KeyFormat::Base58,
            label: None,
            imported_at: chrono::Utc::now().to_rfc3339(),
            balance_snapshot: 500,
        };
        m.import_account(&id2, acct2).unwrap();
        let mm_accounts = m.imported_from(&SourceWallet::MetaMask);
        assert_eq!(mm_accounts.len(), 1);
    }

    #[test]
    fn test_find_by_original_address() {
        let mut m = WalletMigrator::new();
        let id = m.start_migration(SourceWallet::Ledger);
        let acct = sample_account(SourceWallet::Ledger);
        m.import_account(&id, acct).unwrap();
        let found = m.find_by_original_address("0xabc123");
        assert!(found.is_some());
        assert_eq!(found.unwrap().new_address, "evap1xyz789");
    }

    #[test]
    fn test_find_by_original_address_missing() {
        let m = WalletMigrator::new();
        assert!(m.find_by_original_address("nonexistent").is_none());
    }

    #[test]
    fn test_validate_hex() {
        let m = WalletMigrator::new();
        assert!(m.validate_key_format("abcdef01", &KeyFormat::Hex));
        assert!(!m.validate_key_format("xyz", &KeyFormat::Hex));
        assert!(!m.validate_key_format("abc", &KeyFormat::Hex)); // odd length
        assert!(!m.validate_key_format("", &KeyFormat::Hex));
    }

    #[test]
    fn test_validate_base58() {
        let m = WalletMigrator::new();
        assert!(m.validate_key_format(
            "5HueCGU8rMjxEXxiPuD5BDku4MkFqeZyd4dZ1jvhTVqvbTLvyTJ",
            &KeyFormat::Base58
        ));
        assert!(!m.validate_key_format("0OIl", &KeyFormat::Base58)); // invalid base58 chars
        assert!(!m.validate_key_format("", &KeyFormat::Base58));
    }

    #[test]
    fn test_validate_mnemonic12() {
        let m = WalletMigrator::new();
        let twelve = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        assert!(m.validate_key_format(twelve, &KeyFormat::Mnemonic12));
        assert!(!m.validate_key_format("only three words", &KeyFormat::Mnemonic12));
    }

    #[test]
    fn test_validate_mnemonic24() {
        let m = WalletMigrator::new();
        let twenty_four = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        assert!(m.validate_key_format(twenty_four, &KeyFormat::Mnemonic24));
        assert!(!m.validate_key_format("not enough words", &KeyFormat::Mnemonic24));
    }

    #[test]
    fn test_validate_bech32() {
        let m = WalletMigrator::new();
        assert!(m.validate_key_format("bc1qw508d6q", &KeyFormat::Bech32));
        assert!(!m.validate_key_format("short", &KeyFormat::Bech32)); // no '1' separator or too short
    }

    #[test]
    fn test_supported_sources() {
        let sources = WalletMigrator::supported_sources();
        assert_eq!(sources.len(), 6);
        assert!(sources.contains(&SourceWallet::MetaMask));
        assert!(sources.contains(&SourceWallet::Phantom));
    }

    #[test]
    fn test_stats() {
        let mut m = WalletMigrator::new();
        let id1 = m.start_migration(SourceWallet::MetaMask);
        let id2 = m.start_migration(SourceWallet::Phantom);
        m.complete_migration(&id1).unwrap();
        m.fail_migration(&id2, "timeout").unwrap();
        let stats = m.stats();
        assert_eq!(stats.total_migrations, 2);
        assert_eq!(stats.completed, 1);
        assert_eq!(stats.failed, 1);
    }

    #[test]
    fn test_save_and_load() {
        let path = temp_path("save_load.json");
        let mut m = WalletMigrator::new();
        m.start_migration(SourceWallet::MetaMask);
        m.save(&path).unwrap();

        let loaded = WalletMigrator::load(&path).unwrap();
        assert_eq!(loaded.jobs.len(), 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_or_default_missing_file() {
        let path = temp_path("nonexistent.json");
        let m = WalletMigrator::load_or_default(&path);
        assert!(m.jobs.is_empty());
    }

    #[test]
    fn test_source_wallet_display() {
        assert_eq!(SourceWallet::MetaMask.to_string(), "MetaMask");
        assert_eq!(
            SourceWallet::Custom("MyWallet".to_string()).to_string(),
            "Custom(MyWallet)"
        );
    }
}
