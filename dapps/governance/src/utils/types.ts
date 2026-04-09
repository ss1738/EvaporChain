export type ProposalStatus = "active" | "passed" | "rejected" | "expired" | "evaporated";

export interface Proposal {
  id: string;
  title: string;
  description: string;
  proposer: string;
  energy: number;
  max_energy: number;
  current_energy: number;
  half_life: number;
  decay_percentage: number;
  estimated_expiry: number;
  votes_for: number;
  votes_against: number;
  quorum: number;
  quorum_reached: boolean;
  status: ProposalStatus;
  created_at: number;
  category: string;
}

export interface Vote {
  voter: string;
  proposal_id: string;
  direction: "for" | "against";
  weight: number;
  energy_boost: number;
  timestamp: number;
}

export interface GovernanceStats {
  total_proposals: number;
  active_proposals: number;
  passed_proposals: number;
  evaporated_proposals: number;
  total_votes: number;
  total_energy_spent: number;
  participation_rate: number;
}

export interface Delegation {
  from: string;
  to: string;
  weight: number;
  active: boolean;
}

export interface ChainStatus {
  block_height: number;
  epoch: number;
}
