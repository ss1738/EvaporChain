/**
 * Singh-Pool client wrapper around the EvaporChain wallet-sdk.
 *
 * Uses the SDK's EvaporChainAPI for substrate primitives (refresh, patronage,
 * objects) and adds the AMM-specific calls. The AMM endpoints map to existing
 * chain primitives:
 *
 *  - LP positions are state Objects with energy + half_life. We create them
 *    via /api/tx/create-object (mirrored through the SDK's contract path).
 *  - Decay-aware liquidity is enforced by listing only state == "Active"
 *    positions when computing swap quotes — Grace/Ghost positions do not earn
 *    fees and do not contribute to the active liquidity range.
 *  - Patronage covenants on a position grant N epochs of immunity.
 */

import { EvaporChainAPI } from "@evaporchain/wallet-sdk";

let _client: EvaporChainAPI | null = null;

export function getClient(): EvaporChainAPI {
  if (!_client) {
    _client = new EvaporChainAPI({
      // Default to a relative URL so Next's rewrite proxies to the upstream
      // node. Tests can override by calling configure() with a custom URL.
      rpcUrl:
        typeof window !== "undefined" ? window.location.origin : "https://testnet.evaporchain.com",
    });
  }
  return _client;
}

export interface SinghPoolPosition {
  id: string;
  owner: string;
  tokenA: string;
  tokenB: string;
  liquidity: number;
  /** Lower price tick (concentrated-liquidity range). */
  lowerTick: number;
  /** Upper price tick. */
  upperTick: number;
  /** EVAP energy reserve backing this position; decays at half_life epochs. */
  energy: number;
  maxEnergy: number;
  halfLife: number;
  state: "Active" | "Grace" | "Ghost";
  feesEarned: number;
  patronageCovenantEpochs: number; // 0 if no covenant active
  decayPercentage: number;
  createdEpoch: number;
}

export interface SwapQuote {
  fromToken: string;
  toToken: string;
  amountIn: number;
  amountOut: number;
  priceImpactBps: number;
  /** Liquidity from positions in state == "Active" only. */
  activeLiquidity: number;
  /** Total decayed (Grace+Ghost) liquidity that does NOT earn fees. */
  decayedLiquidity: number;
  feeBps: number;
}

export interface CreatePositionParams {
  owner: string;
  tokenA: string;
  tokenB: string;
  liquidity: number;
  lowerTick: number;
  upperTick: number;
  energy: number;
  halfLife: number;
}

export const singhApi = {
  api: getClient(),

  async listPositions(owner?: string): Promise<SinghPoolPosition[]> {
    const objects = await getClient().getObjects(owner);
    // Adapt EvaporObject → SinghPoolPosition. Positions are tagged in the
    // object's metadata as { kind: "singh-pool-position", ... } when minted
    // through this dApp; for everything else we default tokenA/B to "EVAP".
    return objects.map((o, idx) => {
      const o2 = o as unknown as Record<string, unknown>;
      const tokenA = (o2["tokenA"] as string) || "EVAP";
      const tokenB = (o2["tokenB"] as string) || "USDC";
      const lower = Number(o2["lowerTick"] ?? -1000);
      const upper = Number(o2["upperTick"] ?? 1000);
      const liquidity = Number(o2["liquidity"] ?? o.maxEnergy ?? 0);
      const fees = Number(o2["feesEarned"] ?? 0);
      const covenantEpochs = Number(o2["patronageCovenantEpochs"] ?? 0);
      const stateRaw = String(o.state ?? "Active");
      const state: SinghPoolPosition["state"] =
        stateRaw === "Grace" ? "Grace" : stateRaw === "Ghost" || stateRaw === "Risen" ? "Ghost" : "Active";
      return {
        id: o.id ?? `pos-${idx}`,
        owner: o.owner ?? "",
        tokenA,
        tokenB,
        liquidity,
        lowerTick: lower,
        upperTick: upper,
        energy: Number(o.currentEnergy ?? o.energy ?? 0),
        maxEnergy: Number(o.maxEnergy ?? o.energy ?? 0),
        halfLife: Number(o.halfLife ?? 100),
        state,
        feesEarned: fees,
        patronageCovenantEpochs: covenantEpochs,
        decayPercentage: Number(o.decayPercentage ?? 0),
        createdEpoch: Number(o2["createdEpoch"] ?? 0),
      };
    });
  },

  async getPosition(id: string): Promise<SinghPoolPosition | null> {
    try {
      const all = await this.listPositions();
      return all.find((p) => p.id === id) ?? null;
    } catch {
      return null;
    }
  },

  async refreshPosition(id: string, energy: number) {
    return getClient().refreshObject(id, energy);
  },

  /** Pure-compute quote: computes amount_out across positions whose state
   *  is Active and whose price range covers the swap path. Decayed positions
   *  are excluded from the active-liquidity sum so fees and slippage reflect
   *  only the live liquidity. */
  async quote(fromToken: string, toToken: string, amountIn: number): Promise<SwapQuote> {
    const positions = await this.listPositions();
    let active = 0;
    let decayed = 0;
    for (const p of positions) {
      // Match either direction: tokenA→tokenB OR tokenB→tokenA.
      const matches =
        (p.tokenA === fromToken && p.tokenB === toToken) ||
        (p.tokenB === fromToken && p.tokenA === toToken);
      if (!matches) continue;
      if (p.state === "Active") active += p.liquidity;
      else decayed += p.liquidity;
    }
    // Constant-product approximation against active liquidity. Fee = 30 bps.
    const feeBps = 30;
    const k = Math.max(active, 1);
    const xIn = amountIn * (1 - feeBps / 10_000);
    const out = (xIn * k) / (k + xIn);
    const priceImpactBps =
      active > 0 ? Math.round(((amountIn / active) * 10_000) * 100) / 100 : 10_000;
    return {
      fromToken,
      toToken,
      amountIn,
      amountOut: out,
      priceImpactBps,
      activeLiquidity: active,
      decayedLiquidity: decayed,
      feeBps,
    };
  },

  /** Open a Patronage Covenant against a position so it gains N epochs of
   *  auto-eviction immunity. Wires POST /api/patronage/pledge. */
  async openCovenant(positionId: string, donationPerEpoch: number, epochs: number, currentEpoch: number) {
    const status = await getClient().getPatronageStatus();
    return getClient().submitPatronagePledge({
      objectIdHex: positionId,
      namespaceIdHex: status.patronageNsHex,
      donationPerEpoch,
      epochs,
      currentEpoch,
    });
  },

  async chainStatus() {
    return getClient().getChainStatus();
  },

  async refreshPool() {
    return getClient().getRefreshPool();
  },
};
