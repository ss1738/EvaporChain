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
import { ENERGY_POOL_SOURCE, strategyToU64, u64ToStrategy } from "./contract";

// EvaporScript-first migration. Per memory:feedback_evaporscript_first the
// pool lifecycle now lives on-chain. The legacy /pools, /pool/:id, and
// /pool/{create,stake,unstake,…} REST endpoints had no server-side handlers
// (verified 2026-05-01 against crates/evaporchain-node/src/api.rs); the
// dapp was effectively calling 404s.
//
// Read path:  GET /api/scripts → filter name=="EnergyPool" → fan-out to
//             GET /api/script/:id, parse the on-chain `state` blob.
// Write path: POST /api/tx/deploy-script (source) + auto-seal background
//             handoff via POST /api/tx/call-script set_metadata, then
//             stake/unstake/record_save against the same contract id.

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

// ── On-chain registry types ──────────────────────────────────────────

interface ScriptListEntry {
  id: number;
  name: string;
  creator: string;
  energy: number;
  half_life: number;
  created_epoch: number;
  evaporated: boolean;
}

interface ScriptDetail extends ScriptListEntry {
  state: Record<string, unknown>;
  last_refreshed: number;
}

interface EnergyPoolState {
  name?: string;
  description?: string;
  creator?: string;
  strategy?: number;
  sealed?: boolean;
  total_staked?: number;
  contributor_count?: number;
  // Maps come back as JSON objects keyed by hex address.
  stakes?: Record<string, number>;
  guardian_points?: Record<string, number>;
  objects_saved?: Record<string, number>;
  joined_epoch?: Record<string, number>;
  last_distribution_epoch?: number;
}

function scriptToPool(detail: ScriptDetail): Pool | null {
  const s = detail.state as EnergyPoolState;
  if (!s.sealed) return null;
  const liveStakers = Object.entries(s.stakes ?? {}).filter(
    ([_, v]) => Number(v) > 0,
  );
  const liveTotal = liveStakers.reduce((sum, [, v]) => sum + Number(v), 0);
  const cumulative = Number(s.total_staked ?? 0);
  // Health: live total over historical staked. 100% means nothing has
  // been unstaked. Honest UI: when nobody has unstaked yet, we get 1.0.
  const health =
    cumulative > 0 ? Math.min(100, (liveTotal / cumulative) * 100) : 0;
  return {
    id: String(detail.id),
    name: s.name ?? "",
    description: s.description ?? "",
    creator: s.creator ?? detail.creator,
    strategy: u64ToStrategy(Number(s.strategy ?? 0)) as DistributionStrategy,
    total_energy: liveTotal,
    contributor_count: Number(s.contributor_count ?? liveStakers.length),
    protected_objects: [], // Off-chain by design; coordinator owns this.
    health_pct: health,
    created_epoch: detail.created_epoch,
    last_distribution_epoch: Number(s.last_distribution_epoch ?? 0),
  };
}

function canonHex(addr: string): string {
  return addr.trim().replace(/^0x/i, "").toLowerCase();
}

function addressMatches(a: string, b: string): boolean {
  const ca = canonHex(a);
  const cb = canonHex(b);
  if (ca.length === 0 || cb.length === 0) return false;
  const shorter = ca.length < cb.length ? ca : cb;
  const longer = ca.length < cb.length ? cb : ca;
  if (shorter.length < 8) return false;
  return longer.startsWith(shorter);
}

async function listEnergyPoolContracts(): Promise<ScriptDetail[]> {
  const list = await get<{ scripts: ScriptListEntry[]; count: number }>(
    "/scripts",
  );
  const pools = list.scripts.filter((s) => s.name === "EnergyPool").slice(0, 200);
  const details = await Promise.all(
    pools.map((s) => get<ScriptDetail>(`/script/${s.id}`).catch(() => null)),
  );
  return details.filter((d): d is ScriptDetail => d !== null);
}

// ── Deploy + seal helpers ──────────────────────────────────────────

async function deployEnergyPoolContract(energy: number, half_life: number) {
  return post<{ success: boolean; tx_hash?: string; message: string }>(
    "/tx/deploy-script",
    {
      deployer: 1, // address byte; real wallet integration replaces this
      source_code: ENERGY_POOL_SOURCE,
      energy,
      half_life,
    },
  );
}

async function sealEnergyPool(
  contract_id: number,
  name: string,
  description: string,
  strategy: DistributionStrategy,
) {
  return post<{ success: boolean; tx_hash?: string; message: string }>(
    "/tx/call-script",
    {
      caller: 1,
      contract_id,
      method: "set_metadata",
      args: [name, description, strategyToU64(strategy)],
      epoch: 0,
    },
  );
}

async function callPoolMethod(
  contract_id: number,
  caller_byte: number,
  method: string,
  args: unknown[],
) {
  return post<{ success: boolean; tx_hash?: string; message: string }>(
    "/tx/call-script",
    {
      caller: caller_byte,
      contract_id,
      method,
      args,
      epoch: 0,
    },
  );
}

/** Background helper: poll /tx/:hash for finalisation, pull contract_id
 *  from the receipt, then seal. Bounded retry budget. Mirrors the
 *  mortal-messages auto-seal pattern. */
async function sealEnergyPoolWhenReady(
  deploy_tx_hash: string,
  name: string,
  description: string,
  strategy: DistributionStrategy,
): Promise<void> {
  if (!deploy_tx_hash) {
    throw new Error("missing deploy tx_hash; deploy may have been rejected");
  }
  for (let i = 0; i < 30; i++) {
    await new Promise((r) => setTimeout(r, 2000));
    try {
      const status = await get<{
        state: string;
        contract_id?: number;
      }>(`/tx/${deploy_tx_hash}`);
      if (
        (status.state === "finalised" || status.state === "included") &&
        typeof status.contract_id === "number"
      ) {
        await sealEnergyPool(status.contract_id, name, description, strategy);
        return;
      }
      if (status.state === "rejected") {
        throw new Error(`deploy tx rejected: ${deploy_tx_hash}`);
      }
    } catch (e) {
      if (e instanceof Error && e.message.includes("rejected")) throw e;
    }
  }
  throw new Error(
    `deploy tx ${deploy_tx_hash} did not finalise within 60s — seal not issued`,
  );
}

// ── Public API the dapp components use ────────────────────────────

export const api = {
  getStatus: () => get<ChainStatus>("/status"),

  /** Walk the chain registry and return all sealed EnergyPool contracts
   *  as Pool objects. Newest-first. */
  getPools: async (): Promise<Pool[]> => {
    const details = await listEnergyPoolContracts();
    return details
      .map(scriptToPool)
      .filter((p): p is Pool => p !== null)
      .sort((a, b) => b.created_epoch - a.created_epoch);
  },

  /** Single pool by contract id (the dapp's `id` is the script id). */
  getPool: async (id: string): Promise<Pool> => {
    const detail = await get<ScriptDetail>(`/script/${id}`);
    if (detail.name !== "EnergyPool") {
      throw new Error(`script ${id} is not an EnergyPool (name=${detail.name})`);
    }
    const p = scriptToPool(detail);
    if (!p) throw new Error(`EnergyPool ${id} is not yet sealed`);
    return p;
  },

  /** Create + auto-seal a new pool. Returns the deploy tx_hash; the seal
   *  runs in the background once the contract_id resolves on-chain. */
  createPool: async (params: {
    name: string;
    description: string;
    target_objects: string[]; // off-chain; coordinator records these locally
    strategy: DistributionStrategy;
    energy?: number;
    half_life?: number;
  }): Promise<TxResult> => {
    const energy = params.energy ?? 10_000;
    const half_life = params.half_life ?? 10_080; // default ~1 week
    const deploy = await deployEnergyPoolContract(energy, half_life);
    sealEnergyPoolWhenReady(
      deploy.tx_hash ?? "",
      params.name,
      params.description,
      params.strategy,
    ).catch((err) => console.warn("[energy-pool] seal handoff failed:", err));
    return {
      success: deploy.success,
      message: deploy.message,
      tx_hash: deploy.tx_hash,
    };
  },

  stakeEnergy: (poolId: string, amount: number): Promise<TxResult> =>
    callPoolMethod(Number(poolId), 1, "stake", [amount]).then((r) => ({
      success: r.success,
      message: r.message,
      tx_hash: r.tx_hash,
    })),

  unstakeEnergy: (poolId: string, amount: number): Promise<TxResult> =>
    callPoolMethod(Number(poolId), 1, "unstake", [amount]).then((r) => ({
      success: r.success,
      message: r.message,
      tx_hash: r.tx_hash,
    })),

  /** Coordinator-only: record a successful save against the pool. */
  recordSave: (poolId: string): Promise<TxResult> =>
    callPoolMethod(Number(poolId), 1, "record_save", []).then((r) => ({
      success: r.success,
      message: r.message,
      tx_hash: r.tx_hash,
    })),

  /** Per-pool contributor list, derived from the on-chain `stakes` map.
   *  Returns active stakers (balance > 0) sorted by stake descending. */
  getContributors: async (poolId: string): Promise<Contributor[]> => {
    const detail = await get<ScriptDetail>(`/script/${poolId}`);
    const s = detail.state as EnergyPoolState;
    const stakes = s.stakes ?? {};
    const points = s.guardian_points ?? {};
    const saved = s.objects_saved ?? {};
    const joined = s.joined_epoch ?? {};
    const rows: Contributor[] = Object.entries(stakes)
      .filter(([, v]) => Number(v) > 0)
      .map(([addr, v]) => ({
        address: addr,
        staked_energy: Number(v),
        guardian_points: Number(points[addr] ?? 0),
        objects_saved: Number(saved[addr] ?? 0),
        joined_epoch: Number(joined[addr] ?? 0),
        rank: 0, // assigned below after sort
      }));
    rows.sort((a, b) => b.staked_energy - a.staked_energy);
    rows.forEach((r, i) => {
      r.rank = i + 1;
    });
    return rows;
  },

  /** Per-pool activity log. EvaporScript contracts emit events on every
   *  stake / unstake / save; the existing /api/contract/:id/events
   *  endpoint surfaces them. We map those into the dapp's PoolActivity
   *  shape best-effort. */
  getActivity: async (poolId: string): Promise<PoolActivity[]> => {
    try {
      const events = await get<{ events?: { block_number: number; epoch: number; event_name: string; topics?: string[] }[] }>(
        `/contract/${poolId}/events`,
      );
      const list = events.events ?? [];
      return list.map((e, i) => {
        const action: PoolActivity["action"] = e.event_name.includes("stake added")
          ? "stake"
          : e.event_name.includes("stake withdrawn")
            ? "unstake"
            : e.event_name.includes("object saved")
              ? "object_saved"
              : "distribute";
        return {
          id: `${poolId}-${e.block_number}-${i}`,
          pool_id: poolId,
          action,
          address: e.topics?.[0] ?? "",
          amount: 0, // event emit() in EvaporScript doesn't carry args today
          epoch: e.epoch,
          timestamp: 0,
        };
      });
    } catch {
      return [];
    }
  },

  /** Per-user dashboard: walk every EnergyPool contract, sum the user's
   *  stakes and guardian points across them. */
  getDashboard: async (address: string): Promise<UserDashboard> => {
    const details = await listEnergyPoolContracts();
    const userPools = details
      .map((d) => {
        const s = d.state as EnergyPoolState;
        if (!s.sealed) return null;
        const stakes = s.stakes ?? {};
        const points = s.guardian_points ?? {};
        const saved = s.objects_saved ?? {};
        const matchAddr = Object.keys(stakes).find((k) =>
          addressMatches(k, address),
        );
        if (!matchAddr) return null;
        const staked = Number(stakes[matchAddr] ?? 0);
        if (staked <= 0) return null;
        return {
          pool: scriptToPool(d),
          staked_energy: staked,
          guardian_points: Number(points[matchAddr] ?? 0),
          objects_saved: Number(saved[matchAddr] ?? 0),
        };
      })
      .filter((x): x is NonNullable<typeof x> => x !== null && x.pool !== null);

    const total_staked = userPools.reduce((s, x) => s + x.staked_energy, 0);
    const total_guardian_points = userPools.reduce(
      (s, x) => s + x.guardian_points,
      0,
    );
    const objects_saved = userPools.reduce((s, x) => s + x.objects_saved, 0);

    return {
      total_staked,
      total_guardian_points,
      objects_saved,
      pools: userPools.map((x) => ({
        pool_id: x.pool!.id,
        pool_name: x.pool!.name,
        staked_energy: x.staked_energy,
        guardian_points: x.guardian_points,
        health_pct: x.pool!.health_pct,
      })),
    };
  },

  // ── Substrate primitives (unchanged — substrate, not dapp logic) ──
  getRefreshPool: () => get<RefreshPoolStatus>("/refresh_pool"),

  contributeToRefreshPool: async (
    req: RefreshPoolContributeRequest,
  ): Promise<TxResult> => {
    try {
      return await post<TxResult>("/refresh_pool/contribute", req);
    } catch (firstErr) {
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
