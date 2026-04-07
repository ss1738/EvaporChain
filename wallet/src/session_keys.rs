//! Account abstraction — session keys, social recovery, gas sponsorship.
//!
//! # Session Keys
//! Temporary keys with limited permissions and spending caps.
//! Allows dApps to submit transactions without re-prompting for every signature.
//!
//! # Social Recovery
//! Designate guardians who can collectively recover your account.
//! N-of-M guardians must agree to approve a recovery.
//!
//! # Gas Sponsorship
//! Configure a sponsor address that pays gas fees on your behalf.

use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum AbstractionError {
    #[error("session key not found: {0}")]
    SessionKeyNotFound(String),
    #[error("session key expired: {0}")]
    SessionKeyExpired(String),
    #[error("session key spending limit exceeded")]
    SpendingLimitExceeded,
    #[error("guardian not found: {0}")]
    GuardianNotFound(String),
    #[error("duplicate guardian: {0}")]
    DuplicateGuardian(String),
    #[error("recovery not found: {0}")]
    RecoveryNotFound(String),
    #[error("recovery threshold not met: {approvals}/{threshold}")]
    RecoveryThresholdNotMet { approvals: usize, threshold: usize },
    #[error("invalid threshold: {0}")]
    InvalidThreshold(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

// ──────────────────────────── Session Keys ───────────────────────────────

/// A temporary session key with limited permissions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionKey {
    /// Unique session key ID.
    pub id: String,
    /// Human-readable label.
    pub label: String,
    /// The account this key acts on behalf of.
    pub account: String,
    /// Hex-encoded public key of the session key.
    pub public_key: String,
    /// Maximum amount per transaction (0 = unlimited).
    pub max_per_tx: u64,
    /// Total spending limit (0 = unlimited).
    pub total_limit: u64,
    /// Amount already spent.
    pub total_spent: u64,
    /// Allowed operations (e.g., "transfer", "refresh").
    pub allowed_ops: Vec<String>,
    /// Whether active.
    pub active: bool,
    /// Creation timestamp.
    pub created_at: String,
    /// Expiration timestamp.
    pub expires_at: String,
    /// Number of transactions executed.
    pub tx_count: u64,
}

impl SessionKey {
    /// Check if the session key is valid for a given operation and amount.
    pub fn can_execute(&self, operation: &str, amount: u64) -> Result<(), AbstractionError> {
        if !self.active {
            return Err(AbstractionError::SessionKeyNotFound(self.id.clone()));
        }

        // Check expiration
        if let Ok(expires) = chrono::DateTime::parse_from_rfc3339(&self.expires_at) {
            if chrono::Utc::now() > expires {
                return Err(AbstractionError::SessionKeyExpired(self.id.clone()));
            }
        }

        // Check operation allowed
        if !self.allowed_ops.is_empty() && !self.allowed_ops.iter().any(|op| op == operation) {
            return Err(AbstractionError::SessionKeyNotFound(format!(
                "operation '{}' not allowed",
                operation
            )));
        }

        // Check per-tx limit
        if self.max_per_tx > 0 && amount > self.max_per_tx {
            return Err(AbstractionError::SpendingLimitExceeded);
        }

        // Check total limit
        if self.total_limit > 0 && self.total_spent + amount > self.total_limit {
            return Err(AbstractionError::SpendingLimitExceeded);
        }

        Ok(())
    }

    /// Record a transaction execution.
    pub fn record_tx(&mut self, amount: u64) {
        self.total_spent += amount;
        self.tx_count += 1;
    }

    /// Remaining spending allowance.
    pub fn remaining(&self) -> Option<u64> {
        if self.total_limit == 0 {
            None
        } else {
            Some(self.total_limit.saturating_sub(self.total_spent))
        }
    }
}

// ──────────────────────────── Social Recovery ────────────────────────────

/// A guardian designated for social recovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Guardian {
    /// Guardian's address.
    pub address: String,
    /// Human-readable name.
    pub name: String,
    /// When added.
    pub added_at: String,
}

/// A recovery proposal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryProposal {
    /// Proposal ID.
    pub id: String,
    /// New address to recover to.
    pub new_address: String,
    /// Guardians who have approved.
    pub approvals: Vec<String>,
    /// Status.
    pub status: RecoveryStatus,
    /// When proposed.
    pub created_at: String,
    /// When the proposal expires.
    pub expires_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecoveryStatus {
    Pending,
    Approved,
    Executed,
    Expired,
    Cancelled,
}

/// Social recovery configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocialRecovery {
    /// The account being protected.
    pub account: String,
    /// List of guardians.
    pub guardians: Vec<Guardian>,
    /// Number of guardians required for recovery.
    pub threshold: usize,
    /// Recovery delay (hours) after threshold is met.
    pub delay_hours: u64,
    /// Active recovery proposals.
    pub proposals: Vec<RecoveryProposal>,
}

impl SocialRecovery {
    pub fn new(account: &str, threshold: usize, delay_hours: u64) -> Self {
        Self {
            account: account.to_string(),
            guardians: Vec::new(),
            threshold,
            delay_hours,
            proposals: Vec::new(),
        }
    }

    /// Add a guardian.
    pub fn add_guardian(&mut self, address: &str, name: &str) -> Result<(), AbstractionError> {
        if self.guardians.iter().any(|g| g.address == address) {
            return Err(AbstractionError::DuplicateGuardian(address.to_string()));
        }
        self.guardians.push(Guardian {
            address: address.to_string(),
            name: name.to_string(),
            added_at: chrono::Utc::now().to_rfc3339(),
        });
        Ok(())
    }

    /// Remove a guardian.
    pub fn remove_guardian(&mut self, address: &str) -> Result<(), AbstractionError> {
        let before = self.guardians.len();
        self.guardians.retain(|g| g.address != address);
        if self.guardians.len() == before {
            return Err(AbstractionError::GuardianNotFound(address.to_string()));
        }
        Ok(())
    }

    /// Start a recovery proposal.
    pub fn propose_recovery(&mut self, proposer: &str, new_address: &str) -> Result<&RecoveryProposal, AbstractionError> {
        if !self.guardians.iter().any(|g| g.address == proposer) {
            return Err(AbstractionError::GuardianNotFound(proposer.to_string()));
        }

        let now = chrono::Utc::now();
        let expires = now + chrono::Duration::hours(72);
        let id = format!("recv_{}", now.timestamp_millis());

        let proposal = RecoveryProposal {
            id: id.clone(),
            new_address: new_address.to_string(),
            approvals: vec![proposer.to_string()],
            status: if self.threshold <= 1 {
                RecoveryStatus::Approved
            } else {
                RecoveryStatus::Pending
            },
            created_at: now.to_rfc3339(),
            expires_at: expires.to_rfc3339(),
        };

        self.proposals.push(proposal);
        Ok(self.proposals.last().unwrap())
    }

    /// Approve a recovery proposal.
    pub fn approve_recovery(&mut self, proposal_id: &str, guardian: &str) -> Result<&RecoveryProposal, AbstractionError> {
        if !self.guardians.iter().any(|g| g.address == guardian) {
            return Err(AbstractionError::GuardianNotFound(guardian.to_string()));
        }

        let proposal = self.proposals.iter_mut()
            .find(|p| p.id == proposal_id)
            .ok_or_else(|| AbstractionError::RecoveryNotFound(proposal_id.to_string()))?;

        if proposal.approvals.contains(&guardian.to_string()) {
            return Ok(proposal);
        }

        proposal.approvals.push(guardian.to_string());
        if proposal.approvals.len() >= self.threshold {
            proposal.status = RecoveryStatus::Approved;
        }

        Ok(proposal)
    }

    /// Check if recovery is ready to execute.
    pub fn is_recovery_ready(&self, proposal_id: &str) -> bool {
        self.proposals.iter()
            .find(|p| p.id == proposal_id)
            .map(|p| p.status == RecoveryStatus::Approved)
            .unwrap_or(false)
    }

    /// Guardian count.
    pub fn guardian_count(&self) -> usize {
        self.guardians.len()
    }
}

// ──────────────────────────── Gas Sponsorship ────────────────────────────

/// Gas sponsorship configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GasSponsor {
    /// Sponsor address (pays gas for your transactions).
    pub sponsor_address: String,
    /// Maximum gas per transaction the sponsor covers.
    pub max_gas_per_tx: u64,
    /// Daily gas budget from sponsor.
    pub daily_budget: u64,
    /// Gas spent today.
    pub daily_spent: u64,
    /// Date string for daily reset.
    pub date: String,
    /// Whether sponsorship is active.
    pub active: bool,
}

impl GasSponsor {
    pub fn new(sponsor_address: &str, max_gas_per_tx: u64, daily_budget: u64) -> Self {
        Self {
            sponsor_address: sponsor_address.to_string(),
            max_gas_per_tx,
            daily_budget,
            daily_spent: 0,
            date: chrono::Utc::now().format("%Y-%m-%d").to_string(),
            active: true,
        }
    }

    /// Check if sponsor can cover a gas amount.
    pub fn can_sponsor(&mut self, gas_amount: u64) -> bool {
        self.rollover();
        if !self.active {
            return false;
        }
        if gas_amount > self.max_gas_per_tx {
            return false;
        }
        if self.daily_spent + gas_amount > self.daily_budget {
            return false;
        }
        true
    }

    /// Record sponsored gas.
    pub fn record_gas(&mut self, gas_amount: u64) {
        self.rollover();
        self.daily_spent += gas_amount;
    }

    /// Remaining daily budget.
    pub fn remaining(&mut self) -> u64 {
        self.rollover();
        self.daily_budget.saturating_sub(self.daily_spent)
    }

    fn rollover(&mut self) {
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        if self.date != today {
            self.date = today;
            self.daily_spent = 0;
        }
    }
}

// ──────────────────────────── AbstractionStore ───────────────────────────

/// Persistent store for account abstraction features.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbstractionStore {
    pub session_keys: Vec<SessionKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<SocialRecovery>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gas_sponsor: Option<GasSponsor>,
}

impl AbstractionStore {
    pub fn new() -> Self {
        Self {
            session_keys: Vec::new(),
            recovery: None,
            gas_sponsor: None,
        }
    }

    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, AbstractionError> {
        let data = std::fs::read_to_string(path)?;
        let store: AbstractionStore = serde_json::from_str(&data)?;
        Ok(store)
    }

    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), AbstractionError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Create a session key.
    pub fn create_session_key(
        &mut self,
        label: &str,
        account: &str,
        max_per_tx: u64,
        total_limit: u64,
        allowed_ops: Vec<String>,
        duration_hours: u64,
    ) -> &SessionKey {
        let now = chrono::Utc::now();
        let expires = now + chrono::Duration::hours(duration_hours as i64);
        let id = format!("sk_{}", now.timestamp_millis());

        // Generate a simple key ID (in production, would be actual keypair)
        let pk = hex::encode(blake3::hash(format!("{}_{}", id, label).as_bytes()).as_bytes());

        let key = SessionKey {
            id,
            label: label.to_string(),
            account: account.to_string(),
            public_key: pk,
            max_per_tx,
            total_limit,
            total_spent: 0,
            allowed_ops,
            active: true,
            created_at: now.to_rfc3339(),
            expires_at: expires.to_rfc3339(),
            tx_count: 0,
        };

        self.session_keys.push(key);
        self.session_keys.last().unwrap()
    }

    /// Get a session key by ID.
    pub fn get_session_key(&self, id: &str) -> Option<&SessionKey> {
        self.session_keys.iter().find(|k| k.id == id)
    }

    /// Get mutable session key by ID.
    pub fn get_session_key_mut(&mut self, id: &str) -> Option<&mut SessionKey> {
        self.session_keys.iter_mut().find(|k| k.id == id)
    }

    /// Revoke a session key.
    pub fn revoke_session_key(&mut self, id: &str) -> Result<(), AbstractionError> {
        let key = self.session_keys.iter_mut()
            .find(|k| k.id == id)
            .ok_or_else(|| AbstractionError::SessionKeyNotFound(id.to_string()))?;
        key.active = false;
        Ok(())
    }

    /// List active session keys.
    pub fn active_session_keys(&self) -> Vec<&SessionKey> {
        self.session_keys.iter().filter(|k| k.active).collect()
    }

    /// Setup social recovery.
    pub fn setup_recovery(&mut self, account: &str, threshold: usize, delay_hours: u64) -> Result<(), AbstractionError> {
        if threshold == 0 {
            return Err(AbstractionError::InvalidThreshold("threshold must be > 0".into()));
        }
        self.recovery = Some(SocialRecovery::new(account, threshold, delay_hours));
        Ok(())
    }

    /// Setup gas sponsorship.
    pub fn setup_sponsor(&mut self, sponsor_address: &str, max_gas_per_tx: u64, daily_budget: u64) {
        self.gas_sponsor = Some(GasSponsor::new(sponsor_address, max_gas_per_tx, daily_budget));
    }
}

impl Default for AbstractionStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Default path.
pub fn default_abstraction_path() -> std::path::PathBuf {
    crate::config::default_data_dir().join("abstraction.json")
}

// ──────────────────────────── Tests ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_store() -> AbstractionStore {
        let mut store = AbstractionStore::new();
        store.create_session_key("dapp-key", "0xmyaccount", 1000, 10000, vec!["transfer".into()], 24);
        store
    }

    #[test]
    fn test_create_session_key() {
        let store = make_store();
        assert_eq!(store.session_keys.len(), 1);
        let key = &store.session_keys[0];
        assert_eq!(key.label, "dapp-key");
        assert_eq!(key.max_per_tx, 1000);
        assert!(key.active);
    }

    #[test]
    fn test_session_key_can_execute() {
        let store = make_store();
        let key = &store.session_keys[0];
        assert!(key.can_execute("transfer", 500).is_ok());
    }

    #[test]
    fn test_session_key_per_tx_limit() {
        let store = make_store();
        let key = &store.session_keys[0];
        let err = key.can_execute("transfer", 2000).unwrap_err();
        assert!(matches!(err, AbstractionError::SpendingLimitExceeded));
    }

    #[test]
    fn test_session_key_total_limit() {
        let mut store = make_store();
        store.session_keys[0].total_spent = 9500;
        let err = store.session_keys[0].can_execute("transfer", 600).unwrap_err();
        assert!(matches!(err, AbstractionError::SpendingLimitExceeded));
    }

    #[test]
    fn test_session_key_wrong_operation() {
        let store = make_store();
        let key = &store.session_keys[0];
        let err = key.can_execute("deploy_contract", 100).unwrap_err();
        assert!(matches!(err, AbstractionError::SessionKeyNotFound(_)));
    }

    #[test]
    fn test_session_key_record_tx() {
        let mut store = make_store();
        store.session_keys[0].record_tx(500);
        assert_eq!(store.session_keys[0].total_spent, 500);
        assert_eq!(store.session_keys[0].tx_count, 1);
    }

    #[test]
    fn test_session_key_remaining() {
        let store = make_store();
        assert_eq!(store.session_keys[0].remaining(), Some(10000));
    }

    #[test]
    fn test_session_key_remaining_unlimited() {
        let mut store = AbstractionStore::new();
        store.create_session_key("unlimited", "0x1", 0, 0, vec![], 24);
        assert_eq!(store.session_keys[0].remaining(), None);
    }

    #[test]
    fn test_revoke_session_key() {
        let mut store = make_store();
        let id = store.session_keys[0].id.clone();
        store.revoke_session_key(&id).unwrap();
        assert!(!store.session_keys[0].active);
        assert!(store.active_session_keys().is_empty());
    }

    #[test]
    fn test_revoke_not_found() {
        let mut store = AbstractionStore::new();
        let err = store.revoke_session_key("nope").unwrap_err();
        assert!(matches!(err, AbstractionError::SessionKeyNotFound(_)));
    }

    #[test]
    fn test_setup_recovery() {
        let mut store = AbstractionStore::new();
        store.setup_recovery("0xmyaccount", 2, 24).unwrap();
        assert!(store.recovery.is_some());
        assert_eq!(store.recovery.as_ref().unwrap().threshold, 2);
    }

    #[test]
    fn test_setup_recovery_zero_threshold() {
        let mut store = AbstractionStore::new();
        let err = store.setup_recovery("0x1", 0, 24).unwrap_err();
        assert!(matches!(err, AbstractionError::InvalidThreshold(_)));
    }

    #[test]
    fn test_add_guardian() {
        let mut store = AbstractionStore::new();
        store.setup_recovery("0xme", 2, 24).unwrap();
        let recovery = store.recovery.as_mut().unwrap();
        recovery.add_guardian("0xalice", "Alice").unwrap();
        recovery.add_guardian("0xbob", "Bob").unwrap();
        assert_eq!(recovery.guardian_count(), 2);
    }

    #[test]
    fn test_add_duplicate_guardian() {
        let mut store = AbstractionStore::new();
        store.setup_recovery("0xme", 2, 24).unwrap();
        let recovery = store.recovery.as_mut().unwrap();
        recovery.add_guardian("0xalice", "Alice").unwrap();
        let err = recovery.add_guardian("0xalice", "Alice2").unwrap_err();
        assert!(matches!(err, AbstractionError::DuplicateGuardian(_)));
    }

    #[test]
    fn test_remove_guardian() {
        let mut store = AbstractionStore::new();
        store.setup_recovery("0xme", 1, 24).unwrap();
        let recovery = store.recovery.as_mut().unwrap();
        recovery.add_guardian("0xalice", "Alice").unwrap();
        recovery.remove_guardian("0xalice").unwrap();
        assert_eq!(recovery.guardian_count(), 0);
    }

    #[test]
    fn test_propose_recovery() {
        let mut store = AbstractionStore::new();
        store.setup_recovery("0xme", 2, 24).unwrap();
        let recovery = store.recovery.as_mut().unwrap();
        recovery.add_guardian("0xalice", "Alice").unwrap();
        recovery.add_guardian("0xbob", "Bob").unwrap();

        let proposal = recovery.propose_recovery("0xalice", "0xnew_address").unwrap();
        assert_eq!(proposal.status, RecoveryStatus::Pending);
        assert_eq!(proposal.approvals.len(), 1);
    }

    #[test]
    fn test_approve_recovery() {
        let mut store = AbstractionStore::new();
        store.setup_recovery("0xme", 2, 24).unwrap();
        let recovery = store.recovery.as_mut().unwrap();
        recovery.add_guardian("0xalice", "Alice").unwrap();
        recovery.add_guardian("0xbob", "Bob").unwrap();

        let proposal = recovery.propose_recovery("0xalice", "0xnew").unwrap();
        let prop_id = proposal.id.clone();

        let p = recovery.approve_recovery(&prop_id, "0xbob").unwrap();
        assert_eq!(p.status, RecoveryStatus::Approved);
    }

    #[test]
    fn test_is_recovery_ready() {
        let mut store = AbstractionStore::new();
        store.setup_recovery("0xme", 1, 24).unwrap();
        let recovery = store.recovery.as_mut().unwrap();
        recovery.add_guardian("0xalice", "Alice").unwrap();

        let proposal = recovery.propose_recovery("0xalice", "0xnew").unwrap();
        let prop_id = proposal.id.clone();
        assert!(recovery.is_recovery_ready(&prop_id));
    }

    #[test]
    fn test_gas_sponsor() {
        let mut sponsor = GasSponsor::new("0xsponsor", 100_000, 1_000_000);
        assert!(sponsor.can_sponsor(50_000));
        assert!(!sponsor.can_sponsor(200_000)); // exceeds per-tx
        sponsor.record_gas(50_000);
        assert_eq!(sponsor.remaining(), 950_000);
    }

    #[test]
    fn test_gas_sponsor_daily_limit() {
        let mut sponsor = GasSponsor::new("0xsponsor", 1_000_000, 100_000);
        sponsor.record_gas(80_000);
        assert!(!sponsor.can_sponsor(30_000)); // 80k + 30k > 100k daily
    }

    #[test]
    fn test_gas_sponsor_inactive() {
        let mut sponsor = GasSponsor::new("0xsponsor", 100_000, 1_000_000);
        sponsor.active = false;
        assert!(!sponsor.can_sponsor(1));
    }

    #[test]
    fn test_setup_sponsor() {
        let mut store = AbstractionStore::new();
        store.setup_sponsor("0xsponsor", 100_000, 1_000_000);
        assert!(store.gas_sponsor.is_some());
    }

    #[test]
    fn test_json_roundtrip() {
        let mut store = make_store();
        store.setup_recovery("0xme", 2, 24).unwrap();
        store.setup_sponsor("0xsponsor", 100_000, 1_000_000);

        let json = serde_json::to_string_pretty(&store).unwrap();
        let loaded: AbstractionStore = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.session_keys.len(), 1);
        assert!(loaded.recovery.is_some());
        assert!(loaded.gas_sponsor.is_some());
    }

    #[test]
    fn test_file_save_and_load() {
        let dir = std::env::temp_dir().join("evaporchain_abstraction_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("abstraction.json");

        let store = make_store();
        store.save(&path).unwrap();

        let loaded = AbstractionStore::load(&path).unwrap();
        assert_eq!(loaded.session_keys.len(), 1);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_session_key_serializable() {
        let store = make_store();
        let json = serde_json::to_string(&store.session_keys[0]).unwrap();
        assert!(json.contains("\"label\":\"dapp-key\""));
    }
}
