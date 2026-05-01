import type {
  Nft,
  ChainStatus,
  TxResult,
  NftCollection,
  PatronagePledgeRequest,
  PatronagePledgeResponse,
  PatronageStatusResponse,
  PatronageImmunityResponse,
  RefreshPoolStatus,
  DemurrageOwedRequest,
  DemurrageOwedResponse,
  HlwaEffectiveSupplyRequest,
  HlwaEffectiveSupplyResponse,
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

// EvaporScript-first migration. Per memory:feedback_evaporscript_first
// each NFT lives as its own EvaporScript MortalNft contract instance.
// The legacy /nfts, /nft/:id, /nft/collections, /nft/{mint,transfer,refresh}
// REST endpoints had no server-side handlers (verified 2026-05-01); the
// dapp was effectively calling 404s.
//
// Read path: GET /api/scripts → filter name=="MortalNft" → fan-out
//            GET /api/script/:id, parse the on-chain `state` blob.
// Write path:
//   - mint:     POST /api/tx/deploy-script (source=MORTAL_NFT_SOURCE),
//               then auto-seal POST /api/tx/call-script set_metadata.
//   - transfer: POST /api/tx/call-script transfer(to)
//   - refresh:  POST /api/tx/refresh (chain primitive on the contract id)

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

interface MortalNftState {
  name?: string;
  collection?: string;
  metadata?: string;
  sealed?: boolean;
  owner?: string;
  transfer_count?: number;
  last_transfer_epoch?: number;
}

function scriptToNft(detail: ScriptDetail): Nft | null {
  const s = detail.state as MortalNftState;
  if (!s.sealed) return null;
  // The contract doesn't carry max_energy; approximate from current
  // energy so the UI's decay bar has a reference point. Honest UI:
  // never lies about current energy, just doesn't know the deploy-time
  // max without a registry-side enrichment.
  const max_energy = Math.max(detail.energy, detail.energy + 1);
  const decay_pct =
    max_energy > 0 ? Math.max(0, 100 - (detail.energy / max_energy) * 100) : 0;
  const state: "Active" | "Grace" | "Ghost" = detail.evaporated
    ? "Ghost"
    : detail.energy === 0
      ? "Grace"
      : "Active";
  return {
    id: detail.id,
    name: s.name ?? "",
    collection: s.collection ?? "",
    owner: s.owner ?? "",
    metadata_hash: s.metadata ?? "",
    energy: detail.energy,
    max_energy,
    current_energy: detail.energy,
    half_life: detail.half_life,
    minted_epoch: detail.created_epoch,
    last_refreshed: detail.last_refreshed,
    state,
    decay_percentage: decay_pct,
    epochs_remaining: detail.half_life,
    grace_epoch: state === "Grace" ? detail.last_refreshed : null,
    evaporated_epoch: state === "Ghost" ? detail.created_epoch : null,
    ghost_proof: null,
  };
}

async function listMortalNftContracts(): Promise<ScriptDetail[]> {
  const list = await get<{ scripts: ScriptListEntry[]; count: number }>(
    "/scripts",
  );
  const nfts = list.scripts.filter((s) => s.name === "MortalNft").slice(0, 500);
  const details = await Promise.all(
    nfts.map((s) => get<ScriptDetail>(`/script/${s.id}`).catch(() => null)),
  );
  return details.filter((d): d is ScriptDetail => d !== null);
}

async function deployMortalNftContract(energy: number, half_life: number) {
  const { MORTAL_NFT_SOURCE } = await import("./contract");
  return post<{ success: boolean; tx_hash?: string; message: string }>(
    "/tx/deploy-script",
    {
      deployer: 1,
      source_code: MORTAL_NFT_SOURCE,
      energy,
      half_life,
    },
  );
}

async function sealMortalNft(
  contract_id: number,
  name: string,
  collection: string,
  metadata: string,
  recipient_hex: string,
) {
  return post<{ success: boolean; tx_hash?: string; message: string }>(
    "/tx/call-script",
    {
      caller: 1,
      contract_id,
      method: "set_metadata",
      args: [name, collection, metadata, recipient_hex],
      epoch: 0,
    },
  );
}

async function sealMortalNftWhenReady(
  deploy_tx_hash: string,
  name: string,
  collection: string,
  metadata: string,
  recipient_hex: string,
): Promise<void> {
  if (!deploy_tx_hash) {
    throw new Error("missing deploy tx_hash; deploy may have been rejected");
  }
  for (let i = 0; i < 30; i++) {
    await new Promise((r) => setTimeout(r, 2000));
    try {
      const status = await get<{ state: string; contract_id?: number }>(
        `/tx/${deploy_tx_hash}`,
      );
      if (
        (status.state === "finalised" || status.state === "included") &&
        typeof status.contract_id === "number"
      ) {
        await sealMortalNft(status.contract_id, name, collection, metadata, recipient_hex);
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

export const api = {
  getStatus: () => get<ChainStatus>("/status"),

  /** Walk the chain registry and return every sealed MortalNft. */
  getNfts: async (): Promise<Nft[]> => {
    const details = await listMortalNftContracts();
    return details
      .map(scriptToNft)
      .filter((n): n is Nft => n !== null)
      .sort((a, b) => b.minted_epoch - a.minted_epoch);
  },

  getNft: async (id: number): Promise<Nft> => {
    const detail = await get<ScriptDetail>(`/script/${id}`);
    if (detail.name !== "MortalNft") {
      throw new Error(`script ${id} is not a MortalNft (name=${detail.name})`);
    }
    const n = scriptToNft(detail);
    if (!n) throw new Error(`MortalNft ${id} is not yet minted`);
    return n;
  },

  /** Group sealed MortalNfts by `collection` field. */
  getCollections: async (): Promise<NftCollection[]> => {
    const details = await listMortalNftContracts();
    const groups = new Map<string, number[]>();
    for (const d of details) {
      const s = d.state as MortalNftState;
      if (!s.sealed) continue;
      const c = s.collection || "Uncollected";
      const existing = groups.get(c);
      if (existing) existing.push(d.id);
      else groups.set(c, [d.id]);
    }
    return Array.from(groups.entries())
      .map(([name, nft_ids]) => ({ name, count: nft_ids.length, nft_ids }))
      .sort((a, b) => b.count - a.count);
  },

  /** Mint a fresh MortalNft. Deploy-then-seal in one call; the seal
   *  resolves in the background once contract_id is known. */
  mintNft: async (params: {
    name: string;
    collection?: string;
    metadata: string;
    energy: number;
    half_life: number;
    owner?: string;
  }): Promise<TxResult> => {
    const recipient_hex = params.owner ?? "0x0100000000000000000000000000000000000000000000000000000000000000";
    const deploy = await deployMortalNftContract(params.energy, params.half_life);
    sealMortalNftWhenReady(
      deploy.tx_hash ?? "",
      params.name,
      params.collection ?? "",
      params.metadata,
      recipient_hex,
    ).catch((err) => console.warn("[nft-marketplace] seal handoff failed:", err));
    return {
      success: deploy.success,
      message: deploy.message,
      tx_hash: deploy.tx_hash,
    };
  },

  transferNft: (nftId: number, to: string): Promise<TxResult> =>
    post<{ success: boolean; tx_hash?: string; message: string }>(
      "/tx/call-script",
      {
        caller: 1,
        contract_id: nftId,
        method: "transfer",
        args: [to],
        epoch: 0,
      },
    ).then((r) => ({ success: r.success, message: r.message, tx_hash: r.tx_hash })),

  /** Refresh the NFT's energy. The contract id is the underlying
   *  state-object id from the chain's perspective for refresh purposes. */
  refreshNft: (nftId: number, energy: number): Promise<TxResult> =>
    post<{ success: boolean; tx_hash?: string; message: string }>(
      "/tx/refresh",
      {
        object_id: nftId,
        energy_deposit: energy,
      },
    ).then((r) => ({ success: r.success, message: r.message, tx_hash: r.tx_hash })),

  // ── Substrate primitives ──
  // Patronage covenants (object-level immunity from auto-eviction).
  pledgePatronage: (req: PatronagePledgeRequest) =>
    post<PatronagePledgeResponse>("/patronage/pledge", req),
  getPatronageStatus: () => get<PatronageStatusResponse>("/patronage/status"),
  getPatronageImmunity: (objectIdHex: string, epoch: number) =>
    get<PatronageImmunityResponse>(
      `/patronage/immune?object_id_hex=${encodeURIComponent(objectIdHex)}&epoch=${epoch}`,
    ),

  // Refresh pool: protocol-owned bucket of EVAP funding object refreshes.
  getRefreshPool: () => get<RefreshPoolStatus>("/refresh_pool"),

  // Demurrage: pure-compute helper to surface what an account owes.
  computeDemurrage: (req: DemurrageOwedRequest) =>
    post<DemurrageOwedResponse>("/demurrage/owed", req),

  // HLWA: pure-compute λ-decay simulation for wrapped bridge assets.
  hlwaEffectiveSupply: (req: HlwaEffectiveSupplyRequest) =>
    post<HlwaEffectiveSupplyResponse>("/hlwa/effective_supply", req),
};
