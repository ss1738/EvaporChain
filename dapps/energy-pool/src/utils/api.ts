import type {
  Pool,
  Contributor,
  PoolActivity,
  TxResult,
  UserDashboard,
  ChainStatus,
  DistributionStrategy,
} from "./types";

const BASE = "/api";

async function get<T>(path: string): Promise<T> {
  const res = await fetch(`${BASE}${path}`);
  if (!res.ok) throw new Error(`API ${res.status}: ${await res.text()}`);
  return res.json();
}

async function post<T>(path: string, body: unknown): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!res.ok) throw new Error(`API ${res.status}: ${await res.text()}`);
  return res.json();
}

export const api = {
  getStatus: () => get<ChainStatus>("/status"),

  getPools: () => get<Pool[]>("/pools"),

  getPool: (id: string) => get<Pool>(`/pool/${id}`),

  createPool: (params: {
    name: string;
    description: string;
    target_objects: string[];
    strategy: DistributionStrategy;
  }) => post<TxResult>("/pool/create", params),

  stakeEnergy: (poolId: string, amount: number) =>
    post<TxResult>("/pool/stake", { pool_id: poolId, amount }),

  unstakeEnergy: (poolId: string, amount: number) =>
    post<TxResult>("/pool/unstake", { pool_id: poolId, amount }),

  getContributors: (poolId: string) =>
    get<Contributor[]>(`/pool/${poolId}/contributors`),

  getActivity: (poolId: string) =>
    get<PoolActivity[]>(`/pool/${poolId}/activity`),

  getDashboard: (address: string) =>
    get<UserDashboard>(`/pool/dashboard/${address}`),
};
