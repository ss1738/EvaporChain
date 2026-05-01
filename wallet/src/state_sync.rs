//! State synchronization engine — keeps wallet state in sync with the blockchain.
//!
//! Tracks per-account sync progress, detects and resolves conflicts between local
//! and remote state, maintains checkpoints, and provides aggregate sync statistics.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum StateSyncError {
    #[error("account already tracked: {0}")]
    AlreadyTracked(String),
    #[error("account not found: {0}")]
    AccountNotFound(String),
    #[error("conflict not found: {0}")]
    ConflictNotFound(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON parse error: {0}")]
    Parse(#[from] serde_json::Error),
}

// ──────────────────────────── Enums ──────────────────────────────────

/// Current synchronization status of an account.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub enum SyncStatus {
    #[default]
    Synced,
    Syncing,
    Behind,
    Error,
    Offline,
}

/// Strategy used when synchronizing blocks.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub enum SyncMode {
    #[default]
    Full,
    Light,
    Checkpoint,
}

/// How a state conflict should be resolved.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ConflictResolution {
    PreferLocal,
    PreferRemote,
    Manual,
    Latest,
}

// ──────────────────────────── SyncState ──────────────────────────────

/// Per-account synchronization state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncState {
    pub account: String,
    pub local_block: u64,
    pub remote_block: u64,
    pub status: SyncStatus,
    pub last_sync: String,
    pub sync_mode: SyncMode,
    pub blocks_behind: u64,
    pub sync_speed_bps: f64,
    pub error_message: Option<String>,
}

// ──────────────────────────── SyncConflict ───────────────────────────

/// A detected conflict between local and remote state for an account field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConflict {
    pub id: String,
    pub account: String,
    pub field: String,
    pub local_value: String,
    pub remote_value: String,
    pub detected_at: String,
    pub resolved: bool,
    pub resolution: Option<ConflictResolution>,
}

// ──────────────────────────── SyncCheckpoint ─────────────────────────

/// A checkpoint recording blockchain state at a specific block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncCheckpoint {
    pub block_number: u64,
    pub block_hash: String,
    pub state_root: String,
    pub timestamp: String,
    pub accounts_synced: u32,
}

// ──────────────────────────── SyncStats ──────────────────────────────

/// Aggregate synchronization statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncStats {
    pub tracked_accounts: usize,
    pub synced: usize,
    pub behind: usize,
    pub errors: usize,
    pub total_conflicts: usize,
    pub resolved_conflicts: usize,
    pub checkpoints: usize,
    pub avg_blocks_behind: f64,
    pub last_full_sync: Option<String>,
}

// ──────────────────────────── StateSyncManager ───────────────────────

/// Manages synchronization state for all tracked accounts.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StateSyncManager {
    pub accounts: HashMap<String, SyncState>,
    pub conflicts: Vec<SyncConflict>,
    pub checkpoints: Vec<SyncCheckpoint>,
}

impl StateSyncManager {
    /// Create a new, empty manager.
    pub fn new() -> Self {
        Self::default()
    }

    /// Start tracking an account with the given sync mode.
    pub fn track_account(&mut self, account: &str, mode: SyncMode) -> Result<(), StateSyncError> {
        if self.accounts.contains_key(account) {
            return Err(StateSyncError::AlreadyTracked(account.to_string()));
        }
        let state = SyncState {
            account: account.to_string(),
            local_block: 0,
            remote_block: 0,
            status: SyncStatus::Synced,
            last_sync: chrono::Utc::now().to_rfc3339(),
            sync_mode: mode,
            blocks_behind: 0,
            sync_speed_bps: 0.0,
            error_message: None,
        };
        self.accounts.insert(account.to_string(), state);
        Ok(())
    }

    /// Stop tracking an account and return its final state.
    pub fn untrack_account(&mut self, account: &str) -> Result<SyncState, StateSyncError> {
        self.accounts
            .remove(account)
            .ok_or_else(|| StateSyncError::AccountNotFound(account.to_string()))
    }

    /// Update the local block height for an account.
    pub fn update_local_block(&mut self, account: &str, block: u64) -> Result<(), StateSyncError> {
        let state = self
            .accounts
            .get_mut(account)
            .ok_or_else(|| StateSyncError::AccountNotFound(account.to_string()))?;
        state.local_block = block;
        Ok(())
    }

    /// Update the remote block height for an account, recomputing sync gap and status.
    pub fn update_remote_block(&mut self, account: &str, block: u64) -> Result<(), StateSyncError> {
        let state = self
            .accounts
            .get_mut(account)
            .ok_or_else(|| StateSyncError::AccountNotFound(account.to_string()))?;
        state.remote_block = block;
        state.blocks_behind = block.saturating_sub(state.local_block);
        if state.blocks_behind > 0 {
            state.status = SyncStatus::Behind;
        } else {
            state.status = SyncStatus::Synced;
        }
        Ok(())
    }

    /// Instantly sync an account: set local block equal to remote, mark Synced.
    pub fn sync_account(&mut self, account: &str) -> Result<(), StateSyncError> {
        let state = self
            .accounts
            .get_mut(account)
            .ok_or_else(|| StateSyncError::AccountNotFound(account.to_string()))?;
        state.local_block = state.remote_block;
        state.blocks_behind = 0;
        state.status = SyncStatus::Synced;
        state.last_sync = chrono::Utc::now().to_rfc3339();
        state.error_message = None;
        Ok(())
    }

    /// Mark an account as being in an error state.
    pub fn mark_error(&mut self, account: &str, error: &str) -> Result<(), StateSyncError> {
        let state = self
            .accounts
            .get_mut(account)
            .ok_or_else(|| StateSyncError::AccountNotFound(account.to_string()))?;
        state.status = SyncStatus::Error;
        state.error_message = Some(error.to_string());
        Ok(())
    }

    /// Record a new conflict.
    pub fn record_conflict(&mut self, conflict: SyncConflict) {
        self.conflicts.push(conflict);
    }

    /// Resolve a conflict by its id with the chosen resolution strategy.
    pub fn resolve_conflict(
        &mut self,
        conflict_id: &str,
        resolution: ConflictResolution,
    ) -> Result<(), StateSyncError> {
        let conflict = self
            .conflicts
            .iter_mut()
            .find(|c| c.id == conflict_id)
            .ok_or_else(|| StateSyncError::ConflictNotFound(conflict_id.to_string()))?;
        conflict.resolved = true;
        conflict.resolution = Some(resolution);
        Ok(())
    }

    /// Create a new checkpoint at the given block.
    pub fn create_checkpoint(
        &mut self,
        block_number: u64,
        block_hash: &str,
        state_root: &str,
        accounts_synced: u32,
    ) {
        let cp = SyncCheckpoint {
            block_number,
            block_hash: block_hash.to_string(),
            state_root: state_root.to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            accounts_synced,
        };
        self.checkpoints.push(cp);
    }

    /// Return the most recently created checkpoint, if any.
    pub fn latest_checkpoint(&self) -> Option<&SyncCheckpoint> {
        self.checkpoints.last()
    }

    /// Return all accounts that are behind the remote chain.
    pub fn accounts_behind(&self) -> Vec<&SyncState> {
        self.accounts
            .values()
            .filter(|s| s.blocks_behind > 0)
            .collect()
    }

    /// Return all accounts currently in an error state.
    pub fn accounts_in_error(&self) -> Vec<&SyncState> {
        self.accounts
            .values()
            .filter(|s| s.status == SyncStatus::Error)
            .collect()
    }

    /// Compute sync progress for an account as a percentage (0.0–100.0).
    pub fn sync_progress(&self, account: &str) -> Result<f64, StateSyncError> {
        let state = self
            .accounts
            .get(account)
            .ok_or_else(|| StateSyncError::AccountNotFound(account.to_string()))?;
        if state.remote_block == 0 {
            return Ok(100.0);
        }
        Ok((state.local_block as f64 / state.remote_block as f64) * 100.0)
    }

    /// Return all accounts that need syncing (Behind or Error status).
    pub fn needs_sync(&self) -> Vec<&SyncState> {
        self.accounts
            .values()
            .filter(|s| s.status == SyncStatus::Behind || s.status == SyncStatus::Error)
            .collect()
    }

    /// Compute aggregate sync statistics.
    pub fn stats(&self) -> SyncStats {
        let tracked_accounts = self.accounts.len();
        let synced = self
            .accounts
            .values()
            .filter(|s| s.status == SyncStatus::Synced)
            .count();
        let behind = self
            .accounts
            .values()
            .filter(|s| s.status == SyncStatus::Behind)
            .count();
        let errors = self
            .accounts
            .values()
            .filter(|s| s.status == SyncStatus::Error)
            .count();
        let total_conflicts = self.conflicts.len();
        let resolved_conflicts = self.conflicts.iter().filter(|c| c.resolved).count();
        let checkpoints = self.checkpoints.len();

        let total_behind: u64 = self.accounts.values().map(|s| s.blocks_behind).sum();
        let avg_blocks_behind = if tracked_accounts > 0 {
            total_behind as f64 / tracked_accounts as f64
        } else {
            0.0
        };

        // Find the latest last_sync among all synced accounts as the last full sync time.
        let last_full_sync = self
            .accounts
            .values()
            .filter(|s| s.status == SyncStatus::Synced)
            .map(|s| s.last_sync.clone())
            .max();

        SyncStats {
            tracked_accounts,
            synced,
            behind,
            errors,
            total_conflicts,
            resolved_conflicts,
            checkpoints,
            avg_blocks_behind,
            last_full_sync,
        }
    }

    // ──────────────────────── Persistence ─────────────────────────────

    /// Save the manager state to a JSON file.
    pub fn save(&self, path: &Path) -> Result<(), StateSyncError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Load the manager state from a JSON file.
    pub fn load(path: &Path) -> Result<Self, StateSyncError> {
        let data = std::fs::read_to_string(path)?;
        let mgr: Self = serde_json::from_str(&data)?;
        Ok(mgr)
    }

    /// Load from file or return a default instance if the file is missing or invalid.
    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }
}

// ──────────────────────────── Tests ──────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "evaporchain_state_sync_{}_{}.json",
            std::process::id(),
            name
        ))
    }

    #[test]
    fn test_new_manager_is_empty() {
        let mgr = StateSyncManager::new();
        assert!(mgr.accounts.is_empty());
        assert!(mgr.conflicts.is_empty());
        assert!(mgr.checkpoints.is_empty());
    }

    #[test]
    fn test_track_account() {
        let mut mgr = StateSyncManager::new();
        mgr.track_account("acct1", SyncMode::Full).unwrap();
        assert!(mgr.accounts.contains_key("acct1"));
        assert_eq!(mgr.accounts["acct1"].sync_mode, SyncMode::Full);
    }

    #[test]
    fn test_track_duplicate_account_errors() {
        let mut mgr = StateSyncManager::new();
        mgr.track_account("acct1", SyncMode::Full).unwrap();
        let err = mgr.track_account("acct1", SyncMode::Light).unwrap_err();
        assert!(matches!(err, StateSyncError::AlreadyTracked(_)));
    }

    #[test]
    fn test_untrack_account() {
        let mut mgr = StateSyncManager::new();
        mgr.track_account("acct1", SyncMode::Full).unwrap();
        let state = mgr.untrack_account("acct1").unwrap();
        assert_eq!(state.account, "acct1");
        assert!(!mgr.accounts.contains_key("acct1"));
    }

    #[test]
    fn test_untrack_missing_account_errors() {
        let mut mgr = StateSyncManager::new();
        let err = mgr.untrack_account("nonexistent").unwrap_err();
        assert!(matches!(err, StateSyncError::AccountNotFound(_)));
    }

    #[test]
    fn test_update_local_block() {
        let mut mgr = StateSyncManager::new();
        mgr.track_account("acct1", SyncMode::Full).unwrap();
        mgr.update_local_block("acct1", 100).unwrap();
        assert_eq!(mgr.accounts["acct1"].local_block, 100);
    }

    #[test]
    fn test_update_remote_block_sets_behind() {
        let mut mgr = StateSyncManager::new();
        mgr.track_account("acct1", SyncMode::Full).unwrap();
        mgr.update_local_block("acct1", 50).unwrap();
        mgr.update_remote_block("acct1", 100).unwrap();
        assert_eq!(mgr.accounts["acct1"].blocks_behind, 50);
        assert_eq!(mgr.accounts["acct1"].status, SyncStatus::Behind);
    }

    #[test]
    fn test_update_remote_block_synced_when_equal() {
        let mut mgr = StateSyncManager::new();
        mgr.track_account("acct1", SyncMode::Full).unwrap();
        mgr.update_local_block("acct1", 100).unwrap();
        mgr.update_remote_block("acct1", 100).unwrap();
        assert_eq!(mgr.accounts["acct1"].blocks_behind, 0);
        assert_eq!(mgr.accounts["acct1"].status, SyncStatus::Synced);
    }

    #[test]
    fn test_sync_account() {
        let mut mgr = StateSyncManager::new();
        mgr.track_account("acct1", SyncMode::Full).unwrap();
        mgr.update_remote_block("acct1", 200).unwrap();
        assert_eq!(mgr.accounts["acct1"].status, SyncStatus::Behind);
        mgr.sync_account("acct1").unwrap();
        assert_eq!(mgr.accounts["acct1"].local_block, 200);
        assert_eq!(mgr.accounts["acct1"].blocks_behind, 0);
        assert_eq!(mgr.accounts["acct1"].status, SyncStatus::Synced);
    }

    #[test]
    fn test_mark_error() {
        let mut mgr = StateSyncManager::new();
        mgr.track_account("acct1", SyncMode::Full).unwrap();
        mgr.mark_error("acct1", "connection timeout").unwrap();
        assert_eq!(mgr.accounts["acct1"].status, SyncStatus::Error);
        assert_eq!(
            mgr.accounts["acct1"].error_message.as_deref(),
            Some("connection timeout")
        );
    }

    #[test]
    fn test_record_and_resolve_conflict() {
        let mut mgr = StateSyncManager::new();
        let conflict = SyncConflict {
            id: "c1".to_string(),
            account: "acct1".to_string(),
            field: "balance".to_string(),
            local_value: "100".to_string(),
            remote_value: "200".to_string(),
            detected_at: chrono::Utc::now().to_rfc3339(),
            resolved: false,
            resolution: None,
        };
        mgr.record_conflict(conflict);
        assert_eq!(mgr.conflicts.len(), 1);
        assert!(!mgr.conflicts[0].resolved);

        mgr.resolve_conflict("c1", ConflictResolution::PreferRemote)
            .unwrap();
        assert!(mgr.conflicts[0].resolved);
        assert_eq!(
            mgr.conflicts[0].resolution,
            Some(ConflictResolution::PreferRemote)
        );
    }

    #[test]
    fn test_resolve_missing_conflict_errors() {
        let mut mgr = StateSyncManager::new();
        let err = mgr
            .resolve_conflict("missing", ConflictResolution::Manual)
            .unwrap_err();
        assert!(matches!(err, StateSyncError::ConflictNotFound(_)));
    }

    #[test]
    fn test_create_and_latest_checkpoint() {
        let mut mgr = StateSyncManager::new();
        assert!(mgr.latest_checkpoint().is_none());
        mgr.create_checkpoint(100, "0xabc", "0xdef", 5);
        mgr.create_checkpoint(200, "0x123", "0x456", 10);
        let cp = mgr.latest_checkpoint().unwrap();
        assert_eq!(cp.block_number, 200);
        assert_eq!(cp.block_hash, "0x123");
        assert_eq!(cp.accounts_synced, 10);
    }

    #[test]
    fn test_accounts_behind() {
        let mut mgr = StateSyncManager::new();
        mgr.track_account("a1", SyncMode::Full).unwrap();
        mgr.track_account("a2", SyncMode::Light).unwrap();
        mgr.update_remote_block("a1", 50).unwrap();
        // a2 remains synced (both at 0)
        let behind = mgr.accounts_behind();
        assert_eq!(behind.len(), 1);
        assert_eq!(behind[0].account, "a1");
    }

    #[test]
    fn test_accounts_in_error() {
        let mut mgr = StateSyncManager::new();
        mgr.track_account("a1", SyncMode::Full).unwrap();
        mgr.track_account("a2", SyncMode::Full).unwrap();
        mgr.mark_error("a2", "fail").unwrap();
        let errs = mgr.accounts_in_error();
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].account, "a2");
    }

    #[test]
    fn test_sync_progress_full() {
        let mut mgr = StateSyncManager::new();
        mgr.track_account("a1", SyncMode::Full).unwrap();
        mgr.update_local_block("a1", 100).unwrap();
        mgr.update_remote_block("a1", 100).unwrap();
        let pct = mgr.sync_progress("a1").unwrap();
        assert!((pct - 100.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_sync_progress_partial() {
        let mut mgr = StateSyncManager::new();
        mgr.track_account("a1", SyncMode::Full).unwrap();
        mgr.update_local_block("a1", 50).unwrap();
        mgr.update_remote_block("a1", 200).unwrap();
        let pct = mgr.sync_progress("a1").unwrap();
        assert!((pct - 25.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_sync_progress_zero_remote() {
        let mut mgr = StateSyncManager::new();
        mgr.track_account("a1", SyncMode::Full).unwrap();
        let pct = mgr.sync_progress("a1").unwrap();
        assert!((pct - 100.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_needs_sync() {
        let mut mgr = StateSyncManager::new();
        mgr.track_account("a1", SyncMode::Full).unwrap();
        mgr.track_account("a2", SyncMode::Full).unwrap();
        mgr.track_account("a3", SyncMode::Full).unwrap();
        mgr.update_remote_block("a1", 100).unwrap(); // Behind
        mgr.mark_error("a2", "timeout").unwrap(); // Error
                                                  // a3 stays Synced
        let need = mgr.needs_sync();
        assert_eq!(need.len(), 2);
    }

    #[test]
    fn test_stats() {
        let mut mgr = StateSyncManager::new();
        mgr.track_account("a1", SyncMode::Full).unwrap();
        mgr.track_account("a2", SyncMode::Full).unwrap();
        mgr.update_remote_block("a1", 100).unwrap(); // Behind by 100
        mgr.mark_error("a2", "err").unwrap();

        let conflict = SyncConflict {
            id: "c1".to_string(),
            account: "a1".to_string(),
            field: "nonce".to_string(),
            local_value: "1".to_string(),
            remote_value: "2".to_string(),
            detected_at: chrono::Utc::now().to_rfc3339(),
            resolved: false,
            resolution: None,
        };
        mgr.record_conflict(conflict);
        mgr.resolve_conflict("c1", ConflictResolution::Latest)
            .unwrap();
        mgr.create_checkpoint(50, "0x1", "0x2", 2);

        let s = mgr.stats();
        assert_eq!(s.tracked_accounts, 2);
        assert_eq!(s.synced, 0);
        assert_eq!(s.behind, 1);
        assert_eq!(s.errors, 1);
        assert_eq!(s.total_conflicts, 1);
        assert_eq!(s.resolved_conflicts, 1);
        assert_eq!(s.checkpoints, 1);
        assert!((s.avg_blocks_behind - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_save_and_load() {
        let path = temp_path("save_load");
        let mut mgr = StateSyncManager::new();
        mgr.track_account("a1", SyncMode::Checkpoint).unwrap();
        mgr.update_remote_block("a1", 42).unwrap();
        mgr.create_checkpoint(42, "0xaaa", "0xbbb", 1);
        mgr.save(&path).unwrap();

        let loaded = StateSyncManager::load(&path).unwrap();
        assert!(loaded.accounts.contains_key("a1"));
        assert_eq!(loaded.accounts["a1"].remote_block, 42);
        assert_eq!(loaded.checkpoints.len(), 1);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_or_default_missing_file() {
        let path = temp_path("no_such_file");
        let mgr = StateSyncManager::load_or_default(&path);
        assert!(mgr.accounts.is_empty());
    }

    #[test]
    fn test_default_trait() {
        let mgr = StateSyncManager::default();
        assert!(mgr.accounts.is_empty());
        assert!(mgr.conflicts.is_empty());
        assert!(mgr.checkpoints.is_empty());
    }
}
