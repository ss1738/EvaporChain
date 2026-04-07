import type { Nft, ChainStatus, TxResult, NftCollection } from "./types";

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
};
