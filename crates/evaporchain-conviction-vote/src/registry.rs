//! `ConvictionRegistry` — tracks per-voter stake on multiple
//! proposals and runs the per-tick conviction update.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::proposal::{Proposal, ProposalError, ProposalId};

/// 32-byte chain-wide voter handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct VoterId(pub [u8; 32]);

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RegistryError {
    #[error("proposal {0:?} not registered")]
    UnknownProposal(ProposalId),
    #[error("proposal {0:?} already registered")]
    AlreadyRegistered(ProposalId),
    #[error(transparent)]
    Proposal(#[from] ProposalError),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConvictionRegistry {
    proposals: HashMap<ProposalId, Proposal>,
    /// Per-(proposal, voter) stake allocation in micros (energy-
    /// adjusted by the chain's higher layer before passing in).
    stakes: HashMap<(ProposalId, VoterId), u128>,
}

impl ConvictionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, proposal: Proposal) -> Result<(), RegistryError> {
        if self.proposals.contains_key(&proposal.id) {
            return Err(RegistryError::AlreadyRegistered(proposal.id));
        }
        self.proposals.insert(proposal.id, proposal);
        Ok(())
    }

    pub fn get(&self, id: &ProposalId) -> Option<&Proposal> {
        self.proposals.get(id)
    }

    /// Set a voter's allocation on a proposal (energy-adjusted
    /// stake in micros). Replaces any previous allocation.
    pub fn allocate(
        &mut self,
        proposal: ProposalId,
        voter: VoterId,
        stake_micros: u128,
    ) -> Result<(), RegistryError> {
        if !self.proposals.contains_key(&proposal) {
            return Err(RegistryError::UnknownProposal(proposal));
        }
        self.stakes.insert((proposal, voter), stake_micros);
        Ok(())
    }

    /// Sum of all current allocations on a proposal.
    pub fn total_stake_on(&self, proposal: ProposalId) -> u128 {
        self.stakes
            .iter()
            .filter(|((p, _), _)| *p == proposal)
            .map(|(_, s)| *s)
            .sum()
    }

    /// Run one tick of the conviction update on a single proposal.
    /// Caller specifies the energy-adjusted stake total to feed
    /// into the integrator (typically `total_stake_on`).
    pub fn tick(&mut self, proposal: ProposalId, current_tick: u64) -> Result<(), RegistryError> {
        let total = self.total_stake_on(proposal);
        let p = self
            .proposals
            .get_mut(&proposal)
            .ok_or(RegistryError::UnknownProposal(proposal))?;
        p.tick(current_tick, total)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proposal::{ProposalId, ALPHA_MICROS_DEFAULT};

    fn pid(b: u8) -> ProposalId {
        ProposalId([b; 32])
    }
    fn vid(b: u8) -> VoterId {
        VoterId([b; 32])
    }

    fn fresh_proposal(id: ProposalId, threshold: u128) -> Proposal {
        Proposal::new(id, ALPHA_MICROS_DEFAULT, threshold, 0).unwrap()
    }

    #[test]
    fn register_and_lookup() {
        let mut r = ConvictionRegistry::new();
        r.register(fresh_proposal(pid(1), 1_000_000)).unwrap();
        assert!(r.get(&pid(1)).is_some());
    }

    #[test]
    fn duplicate_registration_rejected() {
        let mut r = ConvictionRegistry::new();
        r.register(fresh_proposal(pid(1), 1_000_000)).unwrap();
        let err = r.register(fresh_proposal(pid(1), 5_000_000)).unwrap_err();
        assert_eq!(err, RegistryError::AlreadyRegistered(pid(1)));
    }

    #[test]
    fn allocate_to_unknown_rejected() {
        let mut r = ConvictionRegistry::new();
        let err = r.allocate(pid(99), vid(1), 1000).unwrap_err();
        assert_eq!(err, RegistryError::UnknownProposal(pid(99)));
    }

    #[test]
    fn total_stake_sums_voters() {
        let mut r = ConvictionRegistry::new();
        r.register(fresh_proposal(pid(1), 1_000_000)).unwrap();
        r.allocate(pid(1), vid(1), 100_000).unwrap();
        r.allocate(pid(1), vid(2), 200_000).unwrap();
        r.allocate(pid(1), vid(3), 50_000).unwrap();
        assert_eq!(r.total_stake_on(pid(1)), 350_000);
    }

    #[test]
    fn allocate_replaces_prior_allocation() {
        let mut r = ConvictionRegistry::new();
        r.register(fresh_proposal(pid(1), 1_000_000)).unwrap();
        r.allocate(pid(1), vid(1), 100_000).unwrap();
        r.allocate(pid(1), vid(1), 50_000).unwrap(); // replace
        assert_eq!(r.total_stake_on(pid(1)), 50_000);
    }

    #[test]
    fn tick_uses_total_stake_for_integration() {
        let mut r = ConvictionRegistry::new();
        r.register(fresh_proposal(pid(1), 5_000_000)).unwrap();
        r.allocate(pid(1), vid(1), 500_000).unwrap();
        r.allocate(pid(1), vid(2), 500_000).unwrap();
        // total stake = 1_000_000 → asymptote 10_000_000 (threshold 5M reachable).
        for t in 1..=30 {
            r.tick(pid(1), t).unwrap();
        }
        assert!(r.get(&pid(1)).unwrap().is_passed());
    }

    #[test]
    fn separate_proposals_independent() {
        let mut r = ConvictionRegistry::new();
        r.register(fresh_proposal(pid(1), 5_000_000)).unwrap();
        r.register(fresh_proposal(pid(2), 5_000_000)).unwrap();
        r.allocate(pid(1), vid(1), 1_000_000).unwrap();
        // pid(2) gets nothing.
        for t in 1..=30 {
            r.tick(pid(1), t).unwrap();
            r.tick(pid(2), t).unwrap();
        }
        assert!(r.get(&pid(1)).unwrap().is_passed());
        assert!(!r.get(&pid(2)).unwrap().is_passed());
    }
}
