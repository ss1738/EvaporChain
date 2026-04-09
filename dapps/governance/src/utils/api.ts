import type { Proposal, Vote, GovernanceStats, Delegation, ChainStatus } from "./types";

const BASE = "/api";

async function get<T>(path: string): Promise<T> {
  const res = await fetch(`${BASE}${path}`);
  if (!res.ok) throw new Error(`API ${res.status}`);
  return res.json();
}

async function post<T>(path: string, body: unknown): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!res.ok) throw new Error(`API ${res.status}`);
  return res.json();
}

// ── Proposals ──

export async function getProposals(): Promise<Proposal[]> {
  return get("/dao/proposals");
}

export async function getProposal(id: string): Promise<Proposal> {
  return get(`/dao/proposal/${id}`);
}

export async function createProposal(params: {
  title: string;
  description: string;
  proposer: string;
  energy: number;
  half_life: number;
  category: string;
}): Promise<{ success: boolean; proposal_id: string; message?: string }> {
  return post("/dao/proposals/create", params);
}

// ── Voting ──

export async function vote(params: {
  proposal_id: string;
  voter: string;
  direction: "for" | "against";
  energy_boost?: number;
}): Promise<{ success: boolean; message?: string }> {
  return post("/dao/vote", params);
}

export async function getVotes(proposalId: string): Promise<Vote[]> {
  return get(`/dao/proposal/${proposalId}/votes`);
}

// ── Boost ──

export async function boostProposal(proposalId: string, energy: number): Promise<{ success: boolean; message?: string }> {
  return post("/dao/proposal/boost", { proposal_id: proposalId, energy });
}

// ── Delegation ──

export async function delegate(from: string, to: string, weight: number): Promise<{ success: boolean; message?: string }> {
  return post("/dao/delegate", { from, to, weight });
}

export async function getDelegations(address: string): Promise<Delegation[]> {
  return get(`/dao/delegations/${address}`);
}

// ── Stats ──

export async function getGovernanceStats(): Promise<GovernanceStats> {
  return get("/dao/stats");
}

// ── Chain ──

export async function getStatus(): Promise<ChainStatus> {
  return get("/status");
}
