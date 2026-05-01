import type {
  Pool,
  Contributor,
  PoolActivity,
  TxResult,
  UserDashboard,
  ChainStatus,
  DistributionStrategy,
  RefreshPoolStatus,
  RefreshPoolContributeRequest,
  FeeControllerStatus,
  FeeControllerStepRequest,
  FeeControllerStepResponse,
  PatronageStatusResponse,
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

  // ── Substrate primitives ──
  getRefreshPool: () => get<RefreshPoolStatus>("/refresh_pool"),

  // The substrate exposes /api/refresh_pool/contribute as the "public good"
  // path; we attempt that first and fall back to a transfer into the
  // patronage namespace's vault address (returned via /api/patronage/status)
  // when it isn't wired on the deployed node.
  contributeToRefreshPool: async (req: RefreshPoolContributeRequest): Promise<TxResult> => {
    try {
      return await post<TxResult>("/refresh_pool/contribute", req);
    } catch (firstErr) {
      // Fallback: simulate via transfer to patronage_ns_hex.
      try {
        const patronage = await get<PatronageStatusResponse>("/patronage/status");
        return await post<TxResult>("/tx/transfer", {
          from: req.from,
          to: patronage.patronage_ns_hex,
          amount: req.amount,
          nonce: 0,
          memo: req.memo ?? "refresh_pool/contribute",
        });
      } catch {
        throw firstErr;
      }
    }
  },

  getFeeControllerStatus: () => get<FeeControllerStatus>("/fee_controller/status"),
  stepFeeController: (req: FeeControllerStepRequest) =>
    post<FeeControllerStepResponse>("/fee_controller/step", req),
};
