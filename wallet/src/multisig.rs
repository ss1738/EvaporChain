//! Multi-signature / threshold signing for high-value transactions.
//!
//! Implements an N-of-M approval workflow:
//! 1. Create a multisig group (set of signers + threshold)
//! 2. Propose a transaction (any member can propose)
//! 3. Approve the proposal (each signer signs independently)
//! 4. Execute once threshold is met
//!
//! Proposals and groups are persisted to JSON.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ──────────────────────────── Error ────────────────────────────────────

#[derive(Debug, Error)]
pub enum MultisigError {
    #[error("group not found: {0}")]
    GroupNotFound(String),

    #[error("proposal not found: {0}")]
    ProposalNotFound(String),

    #[error("signer not in group: {0}")]
    NotAMember(String),

    #[error("already approved by: {0}")]
    AlreadyApproved(String),

    #[error("threshold not met: have {approvals}/{threshold}")]
    ThresholdNotMet { approvals: usize, threshold: usize },

    #[error("invalid threshold: {threshold} > {members} members")]
    InvalidThreshold { threshold: usize, members: usize },

    #[error("proposal already executed")]
    AlreadyExecuted,

    #[error("proposal expired")]
    Expired,

    #[error("group already exists: {0}")]
    GroupExists(String),

    #[error("need at least 2 members for multisig")]
    TooFewMembers,

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

// ──────────────────────────── Types ──────────────────────────────────────

/// A multisig group — N-of-M required to approve.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultisigGroup {
    /// Unique name for this group.
    pub name: String,
    /// Member addresses.
    pub members: Vec<String>,
    /// Number of approvals required.
    pub threshold: usize,
    /// Creation timestamp.
    pub created_at: String,
}

/// Status of a proposal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProposalStatus {
    Pending,
    Approved,
    Executed,
    Expired,
    Rejected,
}

/// Type of transaction being proposed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ProposedTx {
    Transfer { to: String, amount: u64 },
    Refresh { object_id: String, energy: u64 },
}

impl ProposedTx {
    /// Human-readable description.
    pub fn describe(&self) -> String {
        match self {
            ProposedTx::Transfer { to, amount } => {
                format!("Transfer {} EVAP to {}", amount, to)
            }
            ProposedTx::Refresh { object_id, energy } => {
                format!("Refresh {} with {} energy", object_id, energy)
            }
        }
    }
}

/// A multisig transaction proposal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proposal {
    /// Unique proposal ID.
    pub id: String,
    /// Group this proposal belongs to.
    pub group_name: String,
    /// Who proposed it.
    pub proposer: String,
    /// The transaction details.
    pub tx: ProposedTx,
    /// Addresses that have approved.
    pub approvals: Vec<String>,
    /// Current status.
    pub status: ProposalStatus,
    /// Creation timestamp.
    pub created_at: String,
    /// Expiration timestamp (ISO 8601).
    pub expires_at: String,
    /// Optional memo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memo: Option<String>,
}

// ──────────────────────────── MultisigManager ────────────────────────────

/// Persistent storage for multisig groups and proposals.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultisigStore {
    pub groups: Vec<MultisigGroup>,
    pub proposals: Vec<Proposal>,
    #[serde(skip)]
    group_index: HashMap<String, usize>,
    #[serde(skip)]
    proposal_index: HashMap<String, usize>,
}

impl MultisigStore {
    /// Create a new empty store.
    pub fn new() -> Self {
        Self {
            groups: Vec::new(),
            proposals: Vec::new(),
            group_index: HashMap::new(),
            proposal_index: HashMap::new(),
        }
    }

    /// Rebuild internal indexes.
    fn rebuild_indexes(&mut self) {
        self.group_index.clear();
        for (i, g) in self.groups.iter().enumerate() {
            self.group_index.insert(g.name.clone(), i);
        }
        self.proposal_index.clear();
        for (i, p) in self.proposals.iter().enumerate() {
            self.proposal_index.insert(p.id.clone(), i);
        }
    }

    /// Load from a JSON file.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, MultisigError> {
        let data = std::fs::read_to_string(path)?;
        let mut store: MultisigStore = serde_json::from_str(&data)?;
        store.rebuild_indexes();
        Ok(store)
    }

    /// Save to a JSON file.
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), MultisigError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    // ── Group Management ────────────────────────────────────────────────

    /// Create a new multisig group.
    pub fn create_group(
        &mut self,
        name: &str,
        members: Vec<String>,
        threshold: usize,
    ) -> Result<&MultisigGroup, MultisigError> {
        if self.group_index.contains_key(name) {
            return Err(MultisigError::GroupExists(name.to_string()));
        }
        if members.len() < 2 {
            return Err(MultisigError::TooFewMembers);
        }
        if threshold > members.len() || threshold == 0 {
            return Err(MultisigError::InvalidThreshold {
                threshold,
                members: members.len(),
            });
        }

        let group = MultisigGroup {
            name: name.to_string(),
            members,
            threshold,
            created_at: chrono::Utc::now().to_rfc3339(),
        };

        let idx = self.groups.len();
        self.groups.push(group);
        self.group_index.insert(name.to_string(), idx);
        Ok(&self.groups[idx])
    }

    /// Get a group by name.
    pub fn get_group(&self, name: &str) -> Option<&MultisigGroup> {
        self.group_index.get(name).map(|&i| &self.groups[i])
    }

    /// List all groups.
    pub fn list_groups(&self) -> &[MultisigGroup] {
        &self.groups
    }

    /// Remove a group (also removes its proposals).
    pub fn remove_group(&mut self, name: &str) -> Result<(), MultisigError> {
        if !self.group_index.contains_key(name) {
            return Err(MultisigError::GroupNotFound(name.to_string()));
        }
        self.groups.retain(|g| g.name != name);
        self.proposals.retain(|p| p.group_name != name);
        self.rebuild_indexes();
        Ok(())
    }

    // ── Proposal Management ─────────────────────────────────────────────

    /// Create a new proposal.
    pub fn propose(
        &mut self,
        group_name: &str,
        proposer: &str,
        tx: ProposedTx,
        memo: Option<&str>,
        expires_hours: u64,
    ) -> Result<&Proposal, MultisigError> {
        let group = self
            .get_group(group_name)
            .ok_or_else(|| MultisigError::GroupNotFound(group_name.to_string()))?;

        // Check proposer is a member
        if !group.members.iter().any(|m| m == proposer) {
            return Err(MultisigError::NotAMember(proposer.to_string()));
        }

        let now = chrono::Utc::now();
        let expires = now + chrono::Duration::hours(expires_hours as i64);
        let id = format!("prop_{}", now.timestamp_millis());

        let proposal = Proposal {
            id: id.clone(),
            group_name: group_name.to_string(),
            proposer: proposer.to_string(),
            tx,
            approvals: vec![proposer.to_string()], // proposer auto-approves
            status: ProposalStatus::Pending,
            created_at: now.to_rfc3339(),
            expires_at: expires.to_rfc3339(),
            memo: memo.map(|s| s.to_string()),
        };

        let idx = self.proposals.len();
        self.proposals.push(proposal);
        self.proposal_index.insert(id, idx);

        // Check if already at threshold (e.g., 1-of-N)
        let group_threshold = self.groups[self.group_index[group_name]].threshold;
        if self.proposals[idx].approvals.len() >= group_threshold {
            self.proposals[idx].status = ProposalStatus::Approved;
        }

        Ok(&self.proposals[idx])
    }

    /// Approve a proposal.
    pub fn approve(
        &mut self,
        proposal_id: &str,
        signer: &str,
    ) -> Result<&Proposal, MultisigError> {
        let idx = *self
            .proposal_index
            .get(proposal_id)
            .ok_or_else(|| MultisigError::ProposalNotFound(proposal_id.to_string()))?;

        let proposal = &self.proposals[idx];

        // Check status
        if proposal.status == ProposalStatus::Executed {
            return Err(MultisigError::AlreadyExecuted);
        }
        if proposal.status == ProposalStatus::Expired {
            return Err(MultisigError::Expired);
        }

        // Check expiration
        if let Ok(expires) = chrono::DateTime::parse_from_rfc3339(&proposal.expires_at) {
            if chrono::Utc::now() > expires {
                self.proposals[idx].status = ProposalStatus::Expired;
                return Err(MultisigError::Expired);
            }
        }

        // Check signer is a member
        let group = self
            .get_group(&proposal.group_name)
            .ok_or_else(|| MultisigError::GroupNotFound(proposal.group_name.clone()))?;

        if !group.members.iter().any(|m| m == signer) {
            return Err(MultisigError::NotAMember(signer.to_string()));
        }

        // Check not already approved
        if proposal.approvals.iter().any(|a| a == signer) {
            return Err(MultisigError::AlreadyApproved(signer.to_string()));
        }

        let threshold = group.threshold;

        // Add approval
        self.proposals[idx].approvals.push(signer.to_string());

        // Check threshold
        if self.proposals[idx].approvals.len() >= threshold {
            self.proposals[idx].status = ProposalStatus::Approved;
        }

        Ok(&self.proposals[idx])
    }

    /// Mark a proposal as executed (after successful on-chain submission).
    pub fn mark_executed(&mut self, proposal_id: &str) -> Result<(), MultisigError> {
        let idx = *self
            .proposal_index
            .get(proposal_id)
            .ok_or_else(|| MultisigError::ProposalNotFound(proposal_id.to_string()))?;

        if self.proposals[idx].status != ProposalStatus::Approved {
            return Err(MultisigError::ThresholdNotMet {
                approvals: self.proposals[idx].approvals.len(),
                threshold: self
                    .get_group(&self.proposals[idx].group_name)
                    .map(|g| g.threshold)
                    .unwrap_or(0),
            });
        }

        self.proposals[idx].status = ProposalStatus::Executed;
        Ok(())
    }

    /// Get a proposal by ID.
    pub fn get_proposal(&self, id: &str) -> Option<&Proposal> {
        self.proposal_index.get(id).map(|&i| &self.proposals[i])
    }

    /// List all proposals for a group.
    pub fn list_proposals(&self, group_name: &str) -> Vec<&Proposal> {
        self.proposals
            .iter()
            .filter(|p| p.group_name == group_name)
            .collect()
    }

    /// List all pending proposals.
    pub fn pending_proposals(&self) -> Vec<&Proposal> {
        self.proposals
            .iter()
            .filter(|p| p.status == ProposalStatus::Pending || p.status == ProposalStatus::Approved)
            .collect()
    }

    /// Check if a proposal is ready to execute.
    pub fn is_ready(&self, proposal_id: &str) -> bool {
        self.get_proposal(proposal_id)
            .map(|p| p.status == ProposalStatus::Approved)
            .unwrap_or(false)
    }
}

impl Default for MultisigStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Default path for multisig store.
pub fn default_multisig_path() -> std::path::PathBuf {
    crate::config::default_data_dir().join("multisig.json")
}

// ──────────────────────────── Tests ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_store() -> MultisigStore {
        let mut store = MultisigStore::new();
        store
            .create_group(
                "treasury",
                vec![
                    "0xalice".to_string(),
                    "0xbob".to_string(),
                    "0xcharlie".to_string(),
                ],
                2,
            )
            .unwrap();
        store
    }

    #[test]
    fn test_create_group() {
        let store = make_store();
        let group = store.get_group("treasury").unwrap();
        assert_eq!(group.members.len(), 3);
        assert_eq!(group.threshold, 2);
    }

    #[test]
    fn test_create_group_duplicate() {
        let mut store = make_store();
        let err = store
            .create_group("treasury", vec!["0xa".into(), "0xb".into()], 1)
            .unwrap_err();
        assert!(matches!(err, MultisigError::GroupExists(_)));
    }

    #[test]
    fn test_create_group_too_few_members() {
        let mut store = MultisigStore::new();
        let err = store
            .create_group("solo", vec!["0xonly".into()], 1)
            .unwrap_err();
        assert!(matches!(err, MultisigError::TooFewMembers));
    }

    #[test]
    fn test_create_group_invalid_threshold() {
        let mut store = MultisigStore::new();
        let err = store
            .create_group("bad", vec!["0xa".into(), "0xb".into()], 3)
            .unwrap_err();
        assert!(matches!(err, MultisigError::InvalidThreshold { .. }));
    }

    #[test]
    fn test_create_group_zero_threshold() {
        let mut store = MultisigStore::new();
        let err = store
            .create_group("bad", vec!["0xa".into(), "0xb".into()], 0)
            .unwrap_err();
        assert!(matches!(err, MultisigError::InvalidThreshold { .. }));
    }

    #[test]
    fn test_propose_auto_approves_proposer() {
        let mut store = make_store();
        let proposal = store
            .propose(
                "treasury",
                "0xalice",
                ProposedTx::Transfer {
                    to: "0xdest".into(),
                    amount: 1000,
                },
                None,
                24,
            )
            .unwrap();

        assert_eq!(proposal.approvals.len(), 1);
        assert_eq!(proposal.approvals[0], "0xalice");
        assert_eq!(proposal.status, ProposalStatus::Pending);
    }

    #[test]
    fn test_propose_non_member_rejected() {
        let mut store = make_store();
        let err = store
            .propose(
                "treasury",
                "0xstranger",
                ProposedTx::Transfer {
                    to: "0xdest".into(),
                    amount: 1000,
                },
                None,
                24,
            )
            .unwrap_err();
        assert!(matches!(err, MultisigError::NotAMember(_)));
    }

    #[test]
    fn test_approve_reaches_threshold() {
        let mut store = make_store();
        let proposal = store
            .propose(
                "treasury",
                "0xalice",
                ProposedTx::Transfer {
                    to: "0xdest".into(),
                    amount: 1000,
                },
                Some("monthly payment"),
                24,
            )
            .unwrap();
        let prop_id = proposal.id.clone();

        // Bob approves — reaches 2-of-3 threshold
        let approved = store.approve(&prop_id, "0xbob").unwrap();
        assert_eq!(approved.approvals.len(), 2);
        assert_eq!(approved.status, ProposalStatus::Approved);
    }

    #[test]
    fn test_approve_duplicate_rejected() {
        let mut store = make_store();
        let proposal = store
            .propose(
                "treasury",
                "0xalice",
                ProposedTx::Transfer {
                    to: "0xdest".into(),
                    amount: 1000,
                },
                None,
                24,
            )
            .unwrap();
        let prop_id = proposal.id.clone();

        let err = store.approve(&prop_id, "0xalice").unwrap_err();
        assert!(matches!(err, MultisigError::AlreadyApproved(_)));
    }

    #[test]
    fn test_approve_non_member_rejected() {
        let mut store = make_store();
        let proposal = store
            .propose(
                "treasury",
                "0xalice",
                ProposedTx::Transfer {
                    to: "0xdest".into(),
                    amount: 1000,
                },
                None,
                24,
            )
            .unwrap();
        let prop_id = proposal.id.clone();

        let err = store.approve(&prop_id, "0xstranger").unwrap_err();
        assert!(matches!(err, MultisigError::NotAMember(_)));
    }

    #[test]
    fn test_mark_executed() {
        let mut store = make_store();
        let proposal = store
            .propose(
                "treasury",
                "0xalice",
                ProposedTx::Transfer {
                    to: "0xdest".into(),
                    amount: 1000,
                },
                None,
                24,
            )
            .unwrap();
        let prop_id = proposal.id.clone();

        store.approve(&prop_id, "0xbob").unwrap();
        store.mark_executed(&prop_id).unwrap();

        let p = store.get_proposal(&prop_id).unwrap();
        assert_eq!(p.status, ProposalStatus::Executed);
    }

    #[test]
    fn test_mark_executed_not_approved() {
        let mut store = make_store();
        let proposal = store
            .propose(
                "treasury",
                "0xalice",
                ProposedTx::Transfer {
                    to: "0xdest".into(),
                    amount: 1000,
                },
                None,
                24,
            )
            .unwrap();
        let prop_id = proposal.id.clone();

        let err = store.mark_executed(&prop_id).unwrap_err();
        assert!(matches!(err, MultisigError::ThresholdNotMet { .. }));
    }

    #[test]
    fn test_execute_already_executed() {
        let mut store = make_store();
        let proposal = store
            .propose(
                "treasury",
                "0xalice",
                ProposedTx::Transfer {
                    to: "0xdest".into(),
                    amount: 1000,
                },
                None,
                24,
            )
            .unwrap();
        let prop_id = proposal.id.clone();

        store.approve(&prop_id, "0xbob").unwrap();
        store.mark_executed(&prop_id).unwrap();

        // Try to approve again
        let err = store.approve(&prop_id, "0xcharlie").unwrap_err();
        assert!(matches!(err, MultisigError::AlreadyExecuted));
    }

    #[test]
    fn test_list_proposals() {
        let mut store = make_store();
        store
            .propose(
                "treasury",
                "0xalice",
                ProposedTx::Transfer {
                    to: "0xdest".into(),
                    amount: 1000,
                },
                None,
                24,
            )
            .unwrap();
        store
            .propose(
                "treasury",
                "0xbob",
                ProposedTx::Refresh {
                    object_id: "0xobj".into(),
                    energy: 500,
                },
                None,
                24,
            )
            .unwrap();

        let proposals = store.list_proposals("treasury");
        assert_eq!(proposals.len(), 2);
    }

    #[test]
    fn test_pending_proposals() {
        let mut store = make_store();
        store
            .propose(
                "treasury",
                "0xalice",
                ProposedTx::Transfer {
                    to: "0xdest".into(),
                    amount: 1000,
                },
                None,
                24,
            )
            .unwrap();

        let pending = store.pending_proposals();
        assert_eq!(pending.len(), 1);
    }

    #[test]
    fn test_is_ready() {
        let mut store = make_store();
        let proposal = store
            .propose(
                "treasury",
                "0xalice",
                ProposedTx::Transfer {
                    to: "0xdest".into(),
                    amount: 1000,
                },
                None,
                24,
            )
            .unwrap();
        let prop_id = proposal.id.clone();

        assert!(!store.is_ready(&prop_id));
        store.approve(&prop_id, "0xbob").unwrap();
        assert!(store.is_ready(&prop_id));
    }

    #[test]
    fn test_remove_group() {
        let mut store = make_store();
        store
            .propose(
                "treasury",
                "0xalice",
                ProposedTx::Transfer {
                    to: "0xdest".into(),
                    amount: 1000,
                },
                None,
                24,
            )
            .unwrap();

        store.remove_group("treasury").unwrap();
        assert!(store.get_group("treasury").is_none());
        assert!(store.list_proposals("treasury").is_empty());
    }

    #[test]
    fn test_remove_group_not_found() {
        let mut store = MultisigStore::new();
        let err = store.remove_group("nope").unwrap_err();
        assert!(matches!(err, MultisigError::GroupNotFound(_)));
    }

    #[test]
    fn test_proposed_tx_describe() {
        let tx = ProposedTx::Transfer {
            to: "0xabc".into(),
            amount: 5000,
        };
        assert!(tx.describe().contains("5000 EVAP"));

        let tx = ProposedTx::Refresh {
            object_id: "0xobj".into(),
            energy: 100,
        };
        assert!(tx.describe().contains("100 energy"));
    }

    #[test]
    fn test_1_of_n_auto_approved() {
        let mut store = MultisigStore::new();
        store
            .create_group(
                "quick",
                vec!["0xalice".into(), "0xbob".into()],
                1,
            )
            .unwrap();

        let proposal = store
            .propose(
                "quick",
                "0xalice",
                ProposedTx::Transfer {
                    to: "0xdest".into(),
                    amount: 100,
                },
                None,
                24,
            )
            .unwrap();

        // 1-of-2: proposer auto-approves, threshold met immediately
        assert_eq!(proposal.status, ProposalStatus::Approved);
    }

    #[test]
    fn test_json_roundtrip() {
        let mut store = make_store();
        store
            .propose(
                "treasury",
                "0xalice",
                ProposedTx::Transfer {
                    to: "0xdest".into(),
                    amount: 1000,
                },
                Some("test memo"),
                24,
            )
            .unwrap();

        let json = serde_json::to_string_pretty(&store).unwrap();
        let mut loaded: MultisigStore = serde_json::from_str(&json).unwrap();
        loaded.rebuild_indexes();

        assert_eq!(loaded.groups.len(), 1);
        assert_eq!(loaded.proposals.len(), 1);
        assert!(loaded.get_group("treasury").is_some());
    }

    #[test]
    fn test_file_save_and_load() {
        let dir = std::env::temp_dir().join("evaporchain_multisig_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("multisig.json");

        let store = make_store();
        store.save(&path).unwrap();

        let loaded = MultisigStore::load(&path).unwrap();
        assert_eq!(loaded.groups.len(), 1);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_list_groups() {
        let store = make_store();
        assert_eq!(store.list_groups().len(), 1);
    }
}
