//! Multi-party shared vault system for EvaporChain.
//!
//! Provides a collaborative vault where multiple members can:
//! 1. Create shared vaults with role-based access
//! 2. Propose actions (transfers, member changes, threshold changes)
//! 3. Approve/reject proposals with threshold-based consensus
//! 4. Execute approved proposals
//! 5. Enforce daily spending limits
//!
//! All data is persisted to JSON.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum SharedVaultError {
    #[error("vault not found: {0}")]
    VaultNotFound(String),

    #[error("vault already exists: {0}")]
    VaultAlreadyExists(String),

    #[error("member not found: {0}")]
    MemberNotFound(String),

    #[error("member already exists: {0}")]
    MemberAlreadyExists(String),

    #[error("proposal not found: {0}")]
    ProposalNotFound(String),

    #[error("proposal not approved")]
    ProposalNotApproved,

    #[error("proposal already executed")]
    ProposalAlreadyExecuted,

    #[error("already voted: {0}")]
    AlreadyVoted(String),

    #[error("spending limit exceeded: requested {requested}, remaining {remaining}")]
    SpendingLimitExceeded { requested: u64, remaining: u64 },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

// ──────────────────────────── Types ──────────────────────────────────────

/// Role within a shared vault.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VaultRole {
    Owner,
    Admin,
    Signer,
    Viewer,
}

/// Status of a vault proposal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProposalStatus {
    Pending,
    Approved,
    Rejected,
    Executed,
    Expired,
}

/// The type of action being proposed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProposalType {
    Transfer {
        to: String,
        amount: u64,
        token: String,
    },
    AddMember {
        member_id: String,
        role: VaultRole,
    },
    RemoveMember {
        member_id: String,
    },
    ChangeThreshold {
        new_threshold: u32,
    },
    Custom {
        description: String,
    },
}

/// A member of a shared vault.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultMember {
    pub id: String,
    pub name: String,
    pub address: String,
    pub role: VaultRole,
    pub added_at: String,
    pub last_active: Option<String>,
}

/// A shared vault with multi-party control.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vault {
    pub id: String,
    pub name: String,
    pub members: HashMap<String, VaultMember>,
    pub threshold: u32,
    pub balance: u64,
    pub created_at: String,
    pub total_proposals: u32,
    pub spending_limit_daily: Option<u64>,
    pub spent_today: u64,
}

/// A proposal within a vault requiring multi-party approval.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultProposal {
    pub id: String,
    pub vault_id: String,
    pub proposer: String,
    pub proposal_type: ProposalType,
    pub status: ProposalStatus,
    pub created_at: String,
    pub expires_at: String,
    pub approvals: Vec<String>,
    pub rejections: Vec<String>,
    pub executed_at: Option<String>,
}

/// Aggregated statistics across all vaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultStats {
    pub total_vaults: usize,
    pub total_members: usize,
    pub total_proposals: usize,
    pub pending_proposals: usize,
    pub executed_proposals: usize,
    pub total_balance: u64,
}

// ──────────────────────────── SharedVaultManager ─────────────────────────

/// Persistent store for shared vaults and proposals.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SharedVaultManager {
    pub vaults: HashMap<String, Vault>,
    pub proposals: HashMap<String, VaultProposal>,
}

impl SharedVaultManager {
    /// Create a new empty manager.
    pub fn new() -> Self {
        Self {
            vaults: HashMap::new(),
            proposals: HashMap::new(),
        }
    }

    // ── Persistence ─────────────────────────────────────────────────────

    /// Load from a JSON file.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, SharedVaultError> {
        let data = std::fs::read_to_string(path)?;
        let store: SharedVaultManager = serde_json::from_str(&data)?;
        Ok(store)
    }

    /// Save to a JSON file.
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), SharedVaultError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Load from file, or return a default empty manager on any error.
    pub fn load_or_default<P: AsRef<Path>>(path: P) -> Self {
        Self::load(path).unwrap_or_default()
    }

    // ── Vault Management ────────────────────────────────────────────────

    /// Create a new vault. Errors if a vault with the same ID already exists.
    pub fn create_vault(&mut self, vault: Vault) -> Result<(), SharedVaultError> {
        if self.vaults.contains_key(&vault.id) {
            return Err(SharedVaultError::VaultAlreadyExists(vault.id.clone()));
        }
        self.vaults.insert(vault.id.clone(), vault);
        Ok(())
    }

    /// Get an immutable reference to a vault.
    pub fn get_vault(&self, id: &str) -> Option<&Vault> {
        self.vaults.get(id)
    }

    /// Get a mutable reference to a vault.
    pub fn get_vault_mut(&mut self, id: &str) -> Option<&mut Vault> {
        self.vaults.get_mut(id)
    }

    /// Add a member to a vault.
    pub fn add_member(
        &mut self,
        vault_id: &str,
        member: VaultMember,
    ) -> Result<(), SharedVaultError> {
        let vault = self
            .vaults
            .get_mut(vault_id)
            .ok_or_else(|| SharedVaultError::VaultNotFound(vault_id.to_string()))?;
        if vault.members.contains_key(&member.id) {
            return Err(SharedVaultError::MemberAlreadyExists(member.id.clone()));
        }
        vault.members.insert(member.id.clone(), member);
        Ok(())
    }

    /// Remove a member from a vault. Returns the removed member.
    pub fn remove_member(
        &mut self,
        vault_id: &str,
        member_id: &str,
    ) -> Result<VaultMember, SharedVaultError> {
        let vault = self
            .vaults
            .get_mut(vault_id)
            .ok_or_else(|| SharedVaultError::VaultNotFound(vault_id.to_string()))?;
        vault
            .members
            .remove(member_id)
            .ok_or_else(|| SharedVaultError::MemberNotFound(member_id.to_string()))
    }

    // ── Proposal Management ─────────────────────────────────────────────

    /// Create a proposal. Errors if the vault does not exist.
    pub fn create_proposal(&mut self, proposal: VaultProposal) -> Result<(), SharedVaultError> {
        if !self.vaults.contains_key(&proposal.vault_id) {
            return Err(SharedVaultError::VaultNotFound(proposal.vault_id.clone()));
        }
        if let Some(vault) = self.vaults.get_mut(&proposal.vault_id) {
            vault.total_proposals += 1;
        }
        self.proposals.insert(proposal.id.clone(), proposal);
        Ok(())
    }

    /// Approve a proposal. If approvals reach the vault threshold, auto-set Approved.
    pub fn approve_proposal(
        &mut self,
        proposal_id: &str,
        member_id: &str,
    ) -> Result<(), SharedVaultError> {
        let proposal = self
            .proposals
            .get_mut(proposal_id)
            .ok_or_else(|| SharedVaultError::ProposalNotFound(proposal_id.to_string()))?;

        if proposal.approvals.contains(&member_id.to_string()) {
            return Err(SharedVaultError::AlreadyVoted(member_id.to_string()));
        }

        proposal.approvals.push(member_id.to_string());

        // Check if threshold is met.
        let threshold = self
            .vaults
            .get(&proposal.vault_id)
            .map(|v| v.threshold)
            .unwrap_or(1);

        if proposal.approvals.len() >= threshold as usize {
            proposal.status = ProposalStatus::Approved;
        }

        Ok(())
    }

    /// Reject a proposal.
    pub fn reject_proposal(
        &mut self,
        proposal_id: &str,
        member_id: &str,
    ) -> Result<(), SharedVaultError> {
        let proposal = self
            .proposals
            .get_mut(proposal_id)
            .ok_or_else(|| SharedVaultError::ProposalNotFound(proposal_id.to_string()))?;

        if proposal.rejections.contains(&member_id.to_string()) {
            return Err(SharedVaultError::AlreadyVoted(member_id.to_string()));
        }

        proposal.rejections.push(member_id.to_string());
        Ok(())
    }

    /// Execute an approved proposal. Errors if the proposal is not in Approved status.
    pub fn execute_proposal(&mut self, proposal_id: &str) -> Result<(), SharedVaultError> {
        let proposal = self
            .proposals
            .get_mut(proposal_id)
            .ok_or_else(|| SharedVaultError::ProposalNotFound(proposal_id.to_string()))?;

        if proposal.status == ProposalStatus::Executed {
            return Err(SharedVaultError::ProposalAlreadyExecuted);
        }

        if proposal.status != ProposalStatus::Approved {
            return Err(SharedVaultError::ProposalNotApproved);
        }

        proposal.status = ProposalStatus::Executed;
        proposal.executed_at = Some(chrono::Utc::now().to_rfc3339());
        Ok(())
    }

    /// Return all pending proposals for a given vault.
    pub fn pending_proposals(&self, vault_id: &str) -> Vec<&VaultProposal> {
        self.proposals
            .values()
            .filter(|p| p.vault_id == vault_id && p.status == ProposalStatus::Pending)
            .collect()
    }

    /// Return all proposals for a given vault (full history).
    pub fn vault_history(&self, vault_id: &str) -> Vec<&VaultProposal> {
        self.proposals
            .values()
            .filter(|p| p.vault_id == vault_id)
            .collect()
    }

    // ── Spending Limits ─────────────────────────────────────────────────

    /// Check whether spending `amount` is within the vault's daily limit.
    /// Returns `true` if within limit (or no limit set), `false` otherwise.
    pub fn check_spending_limit(
        &self,
        vault_id: &str,
        amount: u64,
    ) -> Result<bool, SharedVaultError> {
        let vault = self
            .vaults
            .get(vault_id)
            .ok_or_else(|| SharedVaultError::VaultNotFound(vault_id.to_string()))?;

        match vault.spending_limit_daily {
            Some(limit) => {
                let remaining = limit.saturating_sub(vault.spent_today);
                Ok(amount <= remaining)
            }
            None => Ok(true),
        }
    }

    // ── Queries ─────────────────────────────────────────────────────────

    /// List all vaults.
    pub fn list_vaults(&self) -> Vec<&Vault> {
        self.vaults.values().collect()
    }

    /// Aggregate statistics across all vaults and proposals.
    pub fn stats(&self) -> VaultStats {
        let total_members: usize = self.vaults.values().map(|v| v.members.len()).sum();
        let pending_proposals = self
            .proposals
            .values()
            .filter(|p| p.status == ProposalStatus::Pending)
            .count();
        let executed_proposals = self
            .proposals
            .values()
            .filter(|p| p.status == ProposalStatus::Executed)
            .count();
        let total_balance: u64 = self.vaults.values().map(|v| v.balance).sum();

        VaultStats {
            total_vaults: self.vaults.len(),
            total_members,
            total_proposals: self.proposals.len(),
            pending_proposals,
            executed_proposals,
            total_balance,
        }
    }
}

// ──────────────────────────── Tests ─────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "evaporchain_shared_vault_test_{}_{}",
            std::process::id(),
            name
        ))
    }

    fn make_member(id: &str, role: VaultRole) -> VaultMember {
        VaultMember {
            id: id.to_string(),
            name: format!("Member {}", id),
            address: format!("0x{}", id),
            role,
            added_at: chrono::Utc::now().to_rfc3339(),
            last_active: None,
        }
    }

    fn make_vault(id: &str, threshold: u32) -> Vault {
        let mut members = HashMap::new();
        let owner = make_member("owner1", VaultRole::Owner);
        members.insert(owner.id.clone(), owner);

        Vault {
            id: id.to_string(),
            name: format!("Vault {}", id),
            members,
            threshold,
            balance: 1000,
            created_at: chrono::Utc::now().to_rfc3339(),
            total_proposals: 0,
            spending_limit_daily: None,
            spent_today: 0,
        }
    }

    fn make_proposal(id: &str, vault_id: &str) -> VaultProposal {
        VaultProposal {
            id: id.to_string(),
            vault_id: vault_id.to_string(),
            proposer: "owner1".to_string(),
            proposal_type: ProposalType::Transfer {
                to: "0xrecipient".to_string(),
                amount: 100,
                token: "EVAP".to_string(),
            },
            status: ProposalStatus::Pending,
            created_at: chrono::Utc::now().to_rfc3339(),
            expires_at: "2099-12-31T23:59:59Z".to_string(),
            approvals: Vec::new(),
            rejections: Vec::new(),
            executed_at: None,
        }
    }

    #[test]
    fn test_new_manager_is_empty() {
        let mgr = SharedVaultManager::new();
        assert!(mgr.vaults.is_empty());
        assert!(mgr.proposals.is_empty());
    }

    #[test]
    fn test_create_vault() {
        let mut mgr = SharedVaultManager::new();
        let vault = make_vault("v1", 2);
        assert!(mgr.create_vault(vault).is_ok());
        assert!(mgr.get_vault("v1").is_some());
    }

    #[test]
    fn test_create_vault_duplicate() {
        let mut mgr = SharedVaultManager::new();
        mgr.create_vault(make_vault("v1", 2)).unwrap();
        let result = mgr.create_vault(make_vault("v1", 2));
        assert!(result.is_err());
    }

    #[test]
    fn test_get_vault_mut() {
        let mut mgr = SharedVaultManager::new();
        mgr.create_vault(make_vault("v1", 2)).unwrap();
        let vault = mgr.get_vault_mut("v1").unwrap();
        vault.balance = 5000;
        assert_eq!(mgr.get_vault("v1").unwrap().balance, 5000);
    }

    #[test]
    fn test_add_member() {
        let mut mgr = SharedVaultManager::new();
        mgr.create_vault(make_vault("v1", 2)).unwrap();
        let member = make_member("alice", VaultRole::Signer);
        assert!(mgr.add_member("v1", member).is_ok());
        assert_eq!(mgr.get_vault("v1").unwrap().members.len(), 2);
    }

    #[test]
    fn test_add_member_duplicate() {
        let mut mgr = SharedVaultManager::new();
        mgr.create_vault(make_vault("v1", 2)).unwrap();
        let member = make_member("alice", VaultRole::Signer);
        mgr.add_member("v1", member).unwrap();
        let dup = make_member("alice", VaultRole::Admin);
        assert!(mgr.add_member("v1", dup).is_err());
    }

    #[test]
    fn test_add_member_vault_not_found() {
        let mut mgr = SharedVaultManager::new();
        let member = make_member("alice", VaultRole::Signer);
        assert!(mgr.add_member("nonexistent", member).is_err());
    }

    #[test]
    fn test_remove_member() {
        let mut mgr = SharedVaultManager::new();
        mgr.create_vault(make_vault("v1", 2)).unwrap();
        mgr.add_member("v1", make_member("alice", VaultRole::Signer))
            .unwrap();
        let removed = mgr.remove_member("v1", "alice").unwrap();
        assert_eq!(removed.id, "alice");
        assert_eq!(mgr.get_vault("v1").unwrap().members.len(), 1);
    }

    #[test]
    fn test_remove_member_not_found() {
        let mut mgr = SharedVaultManager::new();
        mgr.create_vault(make_vault("v1", 2)).unwrap();
        assert!(mgr.remove_member("v1", "ghost").is_err());
    }

    #[test]
    fn test_create_proposal() {
        let mut mgr = SharedVaultManager::new();
        mgr.create_vault(make_vault("v1", 2)).unwrap();
        let proposal = make_proposal("p1", "v1");
        assert!(mgr.create_proposal(proposal).is_ok());
        assert_eq!(mgr.get_vault("v1").unwrap().total_proposals, 1);
    }

    #[test]
    fn test_create_proposal_vault_not_found() {
        let mut mgr = SharedVaultManager::new();
        let proposal = make_proposal("p1", "nonexistent");
        assert!(mgr.create_proposal(proposal).is_err());
    }

    #[test]
    fn test_approve_proposal() {
        let mut mgr = SharedVaultManager::new();
        mgr.create_vault(make_vault("v1", 2)).unwrap();
        mgr.create_proposal(make_proposal("p1", "v1")).unwrap();

        mgr.approve_proposal("p1", "alice").unwrap();
        assert_eq!(mgr.proposals["p1"].approvals.len(), 1);
        assert_eq!(mgr.proposals["p1"].status, ProposalStatus::Pending);

        // Second approval should trigger Approved status (threshold = 2).
        mgr.approve_proposal("p1", "bob").unwrap();
        assert_eq!(mgr.proposals["p1"].status, ProposalStatus::Approved);
    }

    #[test]
    fn test_approve_proposal_duplicate() {
        let mut mgr = SharedVaultManager::new();
        mgr.create_vault(make_vault("v1", 2)).unwrap();
        mgr.create_proposal(make_proposal("p1", "v1")).unwrap();
        mgr.approve_proposal("p1", "alice").unwrap();
        assert!(mgr.approve_proposal("p1", "alice").is_err());
    }

    #[test]
    fn test_reject_proposal() {
        let mut mgr = SharedVaultManager::new();
        mgr.create_vault(make_vault("v1", 2)).unwrap();
        mgr.create_proposal(make_proposal("p1", "v1")).unwrap();
        mgr.reject_proposal("p1", "alice").unwrap();
        assert_eq!(mgr.proposals["p1"].rejections.len(), 1);
    }

    #[test]
    fn test_reject_proposal_duplicate() {
        let mut mgr = SharedVaultManager::new();
        mgr.create_vault(make_vault("v1", 2)).unwrap();
        mgr.create_proposal(make_proposal("p1", "v1")).unwrap();
        mgr.reject_proposal("p1", "alice").unwrap();
        assert!(mgr.reject_proposal("p1", "alice").is_err());
    }

    #[test]
    fn test_execute_proposal() {
        let mut mgr = SharedVaultManager::new();
        mgr.create_vault(make_vault("v1", 2)).unwrap();
        mgr.create_proposal(make_proposal("p1", "v1")).unwrap();
        mgr.approve_proposal("p1", "alice").unwrap();
        mgr.approve_proposal("p1", "bob").unwrap();
        assert!(mgr.execute_proposal("p1").is_ok());
        assert_eq!(mgr.proposals["p1"].status, ProposalStatus::Executed);
        assert!(mgr.proposals["p1"].executed_at.is_some());
    }

    #[test]
    fn test_execute_proposal_not_approved() {
        let mut mgr = SharedVaultManager::new();
        mgr.create_vault(make_vault("v1", 2)).unwrap();
        mgr.create_proposal(make_proposal("p1", "v1")).unwrap();
        assert!(mgr.execute_proposal("p1").is_err());
    }

    #[test]
    fn test_execute_proposal_already_executed() {
        let mut mgr = SharedVaultManager::new();
        mgr.create_vault(make_vault("v1", 2)).unwrap();
        mgr.create_proposal(make_proposal("p1", "v1")).unwrap();
        mgr.approve_proposal("p1", "alice").unwrap();
        mgr.approve_proposal("p1", "bob").unwrap();
        mgr.execute_proposal("p1").unwrap();
        assert!(mgr.execute_proposal("p1").is_err());
    }

    #[test]
    fn test_pending_proposals() {
        let mut mgr = SharedVaultManager::new();
        mgr.create_vault(make_vault("v1", 2)).unwrap();
        mgr.create_proposal(make_proposal("p1", "v1")).unwrap();
        mgr.create_proposal(make_proposal("p2", "v1")).unwrap();

        // Approve and execute p2.
        mgr.approve_proposal("p2", "a").unwrap();
        mgr.approve_proposal("p2", "b").unwrap();
        mgr.execute_proposal("p2").unwrap();

        let pending = mgr.pending_proposals("v1");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "p1");
    }

    #[test]
    fn test_vault_history() {
        let mut mgr = SharedVaultManager::new();
        mgr.create_vault(make_vault("v1", 2)).unwrap();
        mgr.create_vault(make_vault("v2", 1)).unwrap();
        mgr.create_proposal(make_proposal("p1", "v1")).unwrap();
        mgr.create_proposal(make_proposal("p2", "v2")).unwrap();

        let history = mgr.vault_history("v1");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].vault_id, "v1");
    }

    #[test]
    fn test_check_spending_limit_no_limit() {
        let mut mgr = SharedVaultManager::new();
        mgr.create_vault(make_vault("v1", 2)).unwrap();
        assert!(mgr.check_spending_limit("v1", 999999).unwrap());
    }

    #[test]
    fn test_check_spending_limit_within() {
        let mut mgr = SharedVaultManager::new();
        let mut vault = make_vault("v1", 2);
        vault.spending_limit_daily = Some(500);
        vault.spent_today = 200;
        mgr.create_vault(vault).unwrap();
        assert!(mgr.check_spending_limit("v1", 300).unwrap());
        assert!(!mgr.check_spending_limit("v1", 301).unwrap());
    }

    #[test]
    fn test_check_spending_limit_vault_not_found() {
        let mgr = SharedVaultManager::new();
        assert!(mgr.check_spending_limit("nonexistent", 100).is_err());
    }

    #[test]
    fn test_list_vaults() {
        let mut mgr = SharedVaultManager::new();
        mgr.create_vault(make_vault("v1", 2)).unwrap();
        mgr.create_vault(make_vault("v2", 1)).unwrap();
        assert_eq!(mgr.list_vaults().len(), 2);
    }

    #[test]
    fn test_stats() {
        let mut mgr = SharedVaultManager::new();
        mgr.create_vault(make_vault("v1", 1)).unwrap();
        mgr.add_member("v1", make_member("alice", VaultRole::Signer))
            .unwrap();
        mgr.create_proposal(make_proposal("p1", "v1")).unwrap();
        mgr.create_proposal(make_proposal("p2", "v1")).unwrap();

        // Execute p1.
        mgr.approve_proposal("p1", "alice").unwrap();
        mgr.execute_proposal("p1").unwrap();

        let s = mgr.stats();
        assert_eq!(s.total_vaults, 1);
        assert_eq!(s.total_members, 2); // owner1 + alice
        assert_eq!(s.total_proposals, 2);
        assert_eq!(s.pending_proposals, 1);
        assert_eq!(s.executed_proposals, 1);
        assert_eq!(s.total_balance, 1000);
    }

    #[test]
    fn test_save_and_load() {
        let path = temp_path("save_load.json");
        let mut mgr = SharedVaultManager::new();
        mgr.create_vault(make_vault("v1", 2)).unwrap();
        mgr.create_proposal(make_proposal("p1", "v1")).unwrap();
        mgr.save(&path).unwrap();

        let loaded = SharedVaultManager::load(&path).unwrap();
        assert_eq!(loaded.vaults.len(), 1);
        assert_eq!(loaded.proposals.len(), 1);
        assert!(loaded.get_vault("v1").is_some());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_or_default_missing_file() {
        let path = temp_path("nonexistent_file.json");
        let _ = std::fs::remove_file(&path);
        let mgr = SharedVaultManager::load_or_default(&path);
        assert!(mgr.vaults.is_empty());
        assert!(mgr.proposals.is_empty());
    }
}
