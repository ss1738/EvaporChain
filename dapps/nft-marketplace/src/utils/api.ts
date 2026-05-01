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

export const api = {
  getStatus: () => get<ChainStatus>("/status"),
  getNfts: () => get<Nft[]>("/nfts"),
  getNft: (id: number) => get<Nft>(`/nft/${id}`),
  getCollections: () => get<NftCollection[]>("/nft/collections"),

  mintNft: (params: {
    name: string;
    collection?: string;
    metadata: string;
    energy: number;
    half_life: number;
    owner?: string;
  }) => post<TxResult>("/nft/mint", params),

  transferNft: (nftId: number, to: string) =>
    post<TxResult>("/nft/transfer", { nft_id: nftId, to }),

  refreshNft: (nftId: number, energy: number) =>
    post<TxResult>("/nft/refresh", { nft_id: nftId, energy }),

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
