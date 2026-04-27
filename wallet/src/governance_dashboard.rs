use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum GovernanceError {
    #[error("Proposal not found: {0}")]
    ProposalNotFound(String),
    #[error("Invalid state transition: {0}")]
    InvalidStateTransition(String),
    #[error("Quorum not reached")]
    QuorumNotReached,
    #[error("Delegation not found")]
    DelegationNotFound,
    #[error("Duplicate proposal ID: {0}")]
    DuplicateProposal(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProposalState {
    Discussion,
    Voting,
    Passed,
    Rejected,
    Executed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VoteChoice {
    For,
    Against,
    Abstain,
}

// ---------------------------------------------------------------------------
// Data structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernanceProposal {
    pub id: String,
    pub title: String,
    pub description: String,
    pub proposer: String,
    pub state: ProposalState,
    pub created_at: String,
    pub voting_start: Option<String>,
    pub voting_end: Option<String>,
    pub executed_at: Option<String>,
    pub votes_for: u64,
    pub votes_against: u64,
    pub votes_abstain: u64,
    pub quorum_required: u64,
    pub total_voting_power: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vote {
    pub proposal_id: String,
    pub voter: String,
    pub choice: VoteChoice,
    pub voting_power: u64,
    pub timestamp: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegationRecord {
    pub from: String,
    pub to: String,
    pub power: u64,
    pub created_at: String,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VotingPowerBreakdown {
    pub address: String,
    pub own_power: u64,
    pub delegated_power: u64,
    pub total_power: u64,
    pub delegation_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernanceStats {
    pub total_proposals: usize,
    pub active_proposals: usize,
    pub passed: usize,
    pub rejected: usize,
    pub total_votes_cast: usize,
    pub total_delegations: usize,
    pub participation_rate: f64,
    pub avg_turnout_pct: f64,
}

// ---------------------------------------------------------------------------
// Main store
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GovernanceDashboard {
    pub proposals: HashMap<String, GovernanceProposal>,
    pub votes: Vec<Vote>,
    pub delegations: Vec<DelegationRecord>,
}

impl GovernanceDashboard {
    pub fn new() -> Self {
        Self::default()
    }

    // -- Proposal lifecycle --------------------------------------------------

    pub fn add_proposal(&mut self, proposal: GovernanceProposal) -> Result<(), GovernanceError> {
        if self.proposals.contains_key(&proposal.id) {
            return Err(GovernanceError::DuplicateProposal(proposal.id.clone()));
        }
        self.proposals.insert(proposal.id.clone(), proposal);
        Ok(())
    }

    pub fn start_voting(&mut self, id: &str, voting_end: &str) -> Result<(), GovernanceError> {
        let proposal = self
            .proposals
            .get_mut(id)
            .ok_or_else(|| GovernanceError::ProposalNotFound(id.to_string()))?;
        if proposal.state != ProposalState::Discussion {
            return Err(GovernanceError::InvalidStateTransition(
                "Proposal must be in Discussion state to start voting".to_string(),
            ));
        }
        proposal.state = ProposalState::Voting;
        proposal.voting_start = Some(chrono::Utc::now().to_rfc3339());
        proposal.voting_end = Some(voting_end.to_string());
        Ok(())
    }

    pub fn cast_vote(&mut self, vote: Vote) -> Result<(), GovernanceError> {
        let proposal = self
            .proposals
            .get_mut(&vote.proposal_id)
            .ok_or_else(|| GovernanceError::ProposalNotFound(vote.proposal_id.clone()))?;
        if proposal.state != ProposalState::Voting {
            return Err(GovernanceError::InvalidStateTransition(
                "Proposal must be in Voting state to cast votes".to_string(),
            ));
        }
        match vote.choice {
            VoteChoice::For => proposal.votes_for += vote.voting_power,
            VoteChoice::Against => proposal.votes_against += vote.voting_power,
            VoteChoice::Abstain => proposal.votes_abstain += vote.voting_power,
        }
        self.votes.push(vote);
        Ok(())
    }

    pub fn finalize_proposal(&mut self, id: &str) -> Result<(), GovernanceError> {
        let proposal = self
            .proposals
            .get_mut(id)
            .ok_or_else(|| GovernanceError::ProposalNotFound(id.to_string()))?;
        if proposal.state != ProposalState::Voting {
            return Err(GovernanceError::InvalidStateTransition(
                "Proposal must be in Voting state to finalize".to_string(),
            ));
        }
        let total_votes = proposal.votes_for + proposal.votes_against + proposal.votes_abstain;
        if total_votes < proposal.quorum_required {
            return Err(GovernanceError::QuorumNotReached);
        }
        if proposal.votes_for > proposal.votes_against {
            proposal.state = ProposalState::Passed;
        } else {
            proposal.state = ProposalState::Rejected;
        }
        Ok(())
    }

    pub fn execute_proposal(&mut self, id: &str) -> Result<(), GovernanceError> {
        let proposal = self
            .proposals
            .get_mut(id)
            .ok_or_else(|| GovernanceError::ProposalNotFound(id.to_string()))?;
        if proposal.state != ProposalState::Passed {
            return Err(GovernanceError::InvalidStateTransition(
                "Proposal must be in Passed state to execute".to_string(),
            ));
        }
        proposal.state = ProposalState::Executed;
        proposal.executed_at = Some(chrono::Utc::now().to_rfc3339());
        Ok(())
    }

    pub fn cancel_proposal(&mut self, id: &str) -> Result<(), GovernanceError> {
        let proposal = self
            .proposals
            .get_mut(id)
            .ok_or_else(|| GovernanceError::ProposalNotFound(id.to_string()))?;
        proposal.state = ProposalState::Cancelled;
        Ok(())
    }

    // -- Queries -------------------------------------------------------------

    pub fn get_proposal(&self, id: &str) -> Option<&GovernanceProposal> {
        self.proposals.get(id)
    }

    pub fn active_proposals(&self) -> Vec<&GovernanceProposal> {
        self.proposals
            .values()
            .filter(|p| p.state == ProposalState::Discussion || p.state == ProposalState::Voting)
            .collect()
    }

    // -- Delegation ----------------------------------------------------------

    pub fn delegate(&mut self, from: &str, to: &str, power: u64) {
        self.delegations.push(DelegationRecord {
            from: from.to_string(),
            to: to.to_string(),
            power,
            created_at: chrono::Utc::now().to_rfc3339(),
            active: true,
        });
    }

    pub fn undelegate(&mut self, from: &str, to: &str) -> Result<(), GovernanceError> {
        let record = self
            .delegations
            .iter_mut()
            .find(|d| d.from == from && d.to == to && d.active);
        match record {
            Some(d) => {
                d.active = false;
                Ok(())
            }
            None => Err(GovernanceError::DelegationNotFound),
        }
    }

    pub fn voting_power(&self, address: &str) -> VotingPowerBreakdown {
        let own_power: u64 = self
            .delegations
            .iter()
            .filter(|d| d.from == address && d.active)
            .map(|d| d.power)
            .sum();

        let delegated_power: u64 = self
            .delegations
            .iter()
            .filter(|d| d.to == address && d.active)
            .map(|d| d.power)
            .sum();

        let delegation_count = self
            .delegations
            .iter()
            .filter(|d| d.to == address && d.active)
            .count() as u32;

        VotingPowerBreakdown {
            address: address.to_string(),
            own_power,
            delegated_power,
            total_power: own_power + delegated_power,
            delegation_count,
        }
    }

    // -- Vote queries --------------------------------------------------------

    pub fn proposal_votes(&self, proposal_id: &str) -> Vec<&Vote> {
        self.votes
            .iter()
            .filter(|v| v.proposal_id == proposal_id)
            .collect()
    }

    pub fn voter_history(&self, address: &str) -> Vec<&Vote> {
        self.votes.iter().filter(|v| v.voter == address).collect()
    }

    pub fn participation_rate(&self, proposal_id: &str) -> Result<f64, GovernanceError> {
        let proposal = self
            .proposals
            .get(proposal_id)
            .ok_or_else(|| GovernanceError::ProposalNotFound(proposal_id.to_string()))?;
        if proposal.total_voting_power == 0 {
            return Ok(0.0);
        }
        let total_votes = proposal.votes_for + proposal.votes_against + proposal.votes_abstain;
        Ok(total_votes as f64 / proposal.total_voting_power as f64)
    }

    pub fn top_delegates(&self, n: usize) -> Vec<VotingPowerBreakdown> {
        let mut addresses: HashMap<&str, ()> = HashMap::new();
        for d in &self.delegations {
            if d.active {
                addresses.insert(&d.to, ());
                addresses.insert(&d.from, ());
            }
        }

        let mut breakdowns: Vec<VotingPowerBreakdown> = addresses
            .keys()
            .map(|addr| self.voting_power(addr))
            .collect();

        breakdowns.sort_by_key(|a| std::cmp::Reverse(a.total_power));
        breakdowns.truncate(n);
        breakdowns
    }

    pub fn stats(&self) -> GovernanceStats {
        let total_proposals = self.proposals.len();
        let active_proposals = self
            .proposals
            .values()
            .filter(|p| p.state == ProposalState::Discussion || p.state == ProposalState::Voting)
            .count();
        let passed = self
            .proposals
            .values()
            .filter(|p| p.state == ProposalState::Passed || p.state == ProposalState::Executed)
            .count();
        let rejected = self
            .proposals
            .values()
            .filter(|p| p.state == ProposalState::Rejected)
            .count();
        let total_votes_cast = self.votes.len();
        let total_delegations = self.delegations.iter().filter(|d| d.active).count();

        let participation_rate = if total_proposals == 0 {
            0.0
        } else {
            let sum: f64 = self
                .proposals
                .keys()
                .filter_map(|id| self.participation_rate(id).ok())
                .sum();
            sum / total_proposals as f64
        };

        let proposals_with_votes: Vec<&GovernanceProposal> = self
            .proposals
            .values()
            .filter(|p| {
                p.state != ProposalState::Discussion && p.total_voting_power > 0
            })
            .collect();
        let avg_turnout_pct = if proposals_with_votes.is_empty() {
            0.0
        } else {
            let sum: f64 = proposals_with_votes
                .iter()
                .map(|p| {
                    let total = p.votes_for + p.votes_against + p.votes_abstain;
                    total as f64 / p.total_voting_power as f64 * 100.0
                })
                .sum();
            sum / proposals_with_votes.len() as f64
        };

        GovernanceStats {
            total_proposals,
            active_proposals,
            passed,
            rejected,
            total_votes_cast,
            total_delegations,
            participation_rate,
            avg_turnout_pct,
        }
    }

    // -- Persistence ---------------------------------------------------------

    pub fn load(path: &Path) -> Result<Self, GovernanceError> {
        let data = std::fs::read_to_string(path)?;
        let dashboard: Self = serde_json::from_str(&data)?;
        Ok(dashboard)
    }

    pub fn save(&self, path: &Path) -> Result<(), GovernanceError> {
        let data = serde_json::to_string_pretty(self)?;
        std::fs::write(path, data)?;
        Ok(())
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

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "governance_dashboard_test_{}_{}",
            std::process::id(),
            name
        ))
    }

    fn make_proposal(id: &str) -> GovernanceProposal {
        GovernanceProposal {
            id: id.to_string(),
            title: format!("Proposal {}", id),
            description: "A test proposal".to_string(),
            proposer: "alice".to_string(),
            state: ProposalState::Discussion,
            created_at: chrono::Utc::now().to_rfc3339(),
            voting_start: None,
            voting_end: None,
            executed_at: None,
            votes_for: 0,
            votes_against: 0,
            votes_abstain: 0,
            quorum_required: 100,
            total_voting_power: 1000,
        }
    }

    fn make_vote(proposal_id: &str, voter: &str, choice: VoteChoice, power: u64) -> Vote {
        Vote {
            proposal_id: proposal_id.to_string(),
            voter: voter.to_string(),
            choice,
            voting_power: power,
            timestamp: chrono::Utc::now().to_rfc3339(),
            reason: None,
        }
    }

    #[test]
    fn test_new_dashboard() {
        let d = GovernanceDashboard::new();
        assert!(d.proposals.is_empty());
        assert!(d.votes.is_empty());
        assert!(d.delegations.is_empty());
    }

    #[test]
    fn test_add_proposal() {
        let mut d = GovernanceDashboard::new();
        assert!(d.add_proposal(make_proposal("p1")).is_ok());
        assert!(d.get_proposal("p1").is_some());
    }

    #[test]
    fn test_duplicate_proposal() {
        let mut d = GovernanceDashboard::new();
        d.add_proposal(make_proposal("p1")).unwrap();
        assert!(d.add_proposal(make_proposal("p1")).is_err());
    }

    #[test]
    fn test_start_voting() {
        let mut d = GovernanceDashboard::new();
        d.add_proposal(make_proposal("p1")).unwrap();
        assert!(d.start_voting("p1", "2026-12-31T00:00:00Z").is_ok());
        assert_eq!(d.get_proposal("p1").unwrap().state, ProposalState::Voting);
        assert!(d.get_proposal("p1").unwrap().voting_start.is_some());
    }

    #[test]
    fn test_start_voting_wrong_state() {
        let mut d = GovernanceDashboard::new();
        d.add_proposal(make_proposal("p1")).unwrap();
        d.start_voting("p1", "2026-12-31T00:00:00Z").unwrap();
        // Already in Voting state, should fail
        assert!(d.start_voting("p1", "2027-01-01T00:00:00Z").is_err());
    }

    #[test]
    fn test_cast_vote() {
        let mut d = GovernanceDashboard::new();
        d.add_proposal(make_proposal("p1")).unwrap();
        d.start_voting("p1", "2026-12-31T00:00:00Z").unwrap();
        let vote = make_vote("p1", "bob", VoteChoice::For, 50);
        assert!(d.cast_vote(vote).is_ok());
        assert_eq!(d.get_proposal("p1").unwrap().votes_for, 50);
    }

    #[test]
    fn test_cast_vote_wrong_state() {
        let mut d = GovernanceDashboard::new();
        d.add_proposal(make_proposal("p1")).unwrap();
        let vote = make_vote("p1", "bob", VoteChoice::For, 50);
        assert!(d.cast_vote(vote).is_err());
    }

    #[test]
    fn test_finalize_proposal_passed() {
        let mut d = GovernanceDashboard::new();
        d.add_proposal(make_proposal("p1")).unwrap();
        d.start_voting("p1", "2026-12-31T00:00:00Z").unwrap();
        d.cast_vote(make_vote("p1", "bob", VoteChoice::For, 80)).unwrap();
        d.cast_vote(make_vote("p1", "carol", VoteChoice::Against, 30)).unwrap();
        assert!(d.finalize_proposal("p1").is_ok());
        assert_eq!(d.get_proposal("p1").unwrap().state, ProposalState::Passed);
    }

    #[test]
    fn test_finalize_proposal_rejected() {
        let mut d = GovernanceDashboard::new();
        d.add_proposal(make_proposal("p1")).unwrap();
        d.start_voting("p1", "2026-12-31T00:00:00Z").unwrap();
        d.cast_vote(make_vote("p1", "bob", VoteChoice::For, 30)).unwrap();
        d.cast_vote(make_vote("p1", "carol", VoteChoice::Against, 80)).unwrap();
        assert!(d.finalize_proposal("p1").is_ok());
        assert_eq!(d.get_proposal("p1").unwrap().state, ProposalState::Rejected);
    }

    #[test]
    fn test_finalize_quorum_not_reached() {
        let mut d = GovernanceDashboard::new();
        d.add_proposal(make_proposal("p1")).unwrap();
        d.start_voting("p1", "2026-12-31T00:00:00Z").unwrap();
        d.cast_vote(make_vote("p1", "bob", VoteChoice::For, 10)).unwrap();
        assert!(d.finalize_proposal("p1").is_err());
    }

    #[test]
    fn test_execute_proposal() {
        let mut d = GovernanceDashboard::new();
        d.add_proposal(make_proposal("p1")).unwrap();
        d.start_voting("p1", "2026-12-31T00:00:00Z").unwrap();
        d.cast_vote(make_vote("p1", "bob", VoteChoice::For, 200)).unwrap();
        d.finalize_proposal("p1").unwrap();
        assert!(d.execute_proposal("p1").is_ok());
        assert_eq!(d.get_proposal("p1").unwrap().state, ProposalState::Executed);
        assert!(d.get_proposal("p1").unwrap().executed_at.is_some());
    }

    #[test]
    fn test_execute_wrong_state() {
        let mut d = GovernanceDashboard::new();
        d.add_proposal(make_proposal("p1")).unwrap();
        assert!(d.execute_proposal("p1").is_err());
    }

    #[test]
    fn test_cancel_proposal() {
        let mut d = GovernanceDashboard::new();
        d.add_proposal(make_proposal("p1")).unwrap();
        assert!(d.cancel_proposal("p1").is_ok());
        assert_eq!(d.get_proposal("p1").unwrap().state, ProposalState::Cancelled);
    }

    #[test]
    fn test_active_proposals() {
        let mut d = GovernanceDashboard::new();
        d.add_proposal(make_proposal("p1")).unwrap();
        d.add_proposal(make_proposal("p2")).unwrap();
        d.add_proposal(make_proposal("p3")).unwrap();
        d.start_voting("p2", "2026-12-31T00:00:00Z").unwrap();
        d.cancel_proposal("p3").unwrap();
        let active = d.active_proposals();
        assert_eq!(active.len(), 2);
    }

    #[test]
    fn test_delegate_and_voting_power() {
        let mut d = GovernanceDashboard::new();
        d.delegate("alice", "bob", 100);
        d.delegate("carol", "bob", 50);
        let bp = d.voting_power("bob");
        assert_eq!(bp.delegated_power, 150);
        assert_eq!(bp.delegation_count, 2);
        assert_eq!(bp.total_power, 150);
    }

    #[test]
    fn test_undelegate() {
        let mut d = GovernanceDashboard::new();
        d.delegate("alice", "bob", 100);
        assert!(d.undelegate("alice", "bob").is_ok());
        let bp = d.voting_power("bob");
        assert_eq!(bp.delegated_power, 0);
    }

    #[test]
    fn test_undelegate_not_found() {
        let mut d = GovernanceDashboard::new();
        assert!(d.undelegate("alice", "bob").is_err());
    }

    #[test]
    fn test_proposal_votes() {
        let mut d = GovernanceDashboard::new();
        d.add_proposal(make_proposal("p1")).unwrap();
        d.start_voting("p1", "2026-12-31T00:00:00Z").unwrap();
        d.cast_vote(make_vote("p1", "bob", VoteChoice::For, 50)).unwrap();
        d.cast_vote(make_vote("p1", "carol", VoteChoice::Against, 30)).unwrap();
        assert_eq!(d.proposal_votes("p1").len(), 2);
    }

    #[test]
    fn test_voter_history() {
        let mut d = GovernanceDashboard::new();
        d.add_proposal(make_proposal("p1")).unwrap();
        d.add_proposal(make_proposal("p2")).unwrap();
        d.start_voting("p1", "end").unwrap();
        d.start_voting("p2", "end").unwrap();
        d.cast_vote(make_vote("p1", "bob", VoteChoice::For, 10)).unwrap();
        d.cast_vote(make_vote("p2", "bob", VoteChoice::Against, 20)).unwrap();
        assert_eq!(d.voter_history("bob").len(), 2);
    }

    #[test]
    fn test_participation_rate() {
        let mut d = GovernanceDashboard::new();
        d.add_proposal(make_proposal("p1")).unwrap();
        d.start_voting("p1", "end").unwrap();
        d.cast_vote(make_vote("p1", "bob", VoteChoice::For, 500)).unwrap();
        let rate = d.participation_rate("p1").unwrap();
        assert!((rate - 0.5).abs() < 1e-9);
    }

    #[test]
    fn test_top_delegates() {
        let mut d = GovernanceDashboard::new();
        d.delegate("a", "b", 100);
        d.delegate("c", "d", 200);
        d.delegate("e", "b", 50);
        let top = d.top_delegates(2);
        assert_eq!(top.len(), 2);
        assert!(top[0].total_power >= top[1].total_power);
    }

    #[test]
    fn test_stats() {
        let mut d = GovernanceDashboard::new();
        d.add_proposal(make_proposal("p1")).unwrap();
        d.add_proposal(make_proposal("p2")).unwrap();
        d.start_voting("p1", "end").unwrap();
        d.cast_vote(make_vote("p1", "bob", VoteChoice::For, 200)).unwrap();
        d.finalize_proposal("p1").unwrap();
        d.delegate("a", "b", 100);
        let s = d.stats();
        assert_eq!(s.total_proposals, 2);
        assert_eq!(s.active_proposals, 1);
        assert_eq!(s.passed, 1);
        assert_eq!(s.total_votes_cast, 1);
        assert_eq!(s.total_delegations, 1);
    }

    #[test]
    fn test_save_and_load() {
        let path = temp_path("save_load.json");
        let mut d = GovernanceDashboard::new();
        d.add_proposal(make_proposal("p1")).unwrap();
        d.delegate("alice", "bob", 100);
        d.save(&path).unwrap();

        let loaded = GovernanceDashboard::load(&path).unwrap();
        assert!(loaded.get_proposal("p1").is_some());
        assert_eq!(loaded.delegations.len(), 1);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_or_default() {
        let path = temp_path("nonexistent.json");
        let _ = std::fs::remove_file(&path);
        let d = GovernanceDashboard::load_or_default(&path);
        assert!(d.proposals.is_empty());
    }
}
