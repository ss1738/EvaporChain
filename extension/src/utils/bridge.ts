/**
 * Multi-chain bridge client for EvaporChain wallet extension.
 * Handles cross-chain asset transfers between EvaporChain, Ethereum, Solana, and Polygon.
 *
 * STUB implementation — connects to placeholder API URLs.
 * Bridge backend will be built as part of the EvaporChain Bridge v1 infrastructure.
 */

const BRIDGE_API = "https://bridge.evaporchain.com/api/v1";

// ── Types ──

export interface Chain {
  id: string;
  name: string;
  symbol: string;
  color: string;
  explorerUrl: string;
  rpcUrl: string;
  icon: string;
}

export interface Asset {
  symbol: string;
  name: string;
  decimals: number;
  nativeChain: string;
  contractAddress?: string;
  logo?: string;
}

export interface BridgeQuote {
  fromChain: string;
  toChain: string;
  asset: string;
  amount: number;
  fee: number;
  feePercent: number;
  estimatedTimeMinutes: number;
  destinationAmount: number;
  exchangeRate: number;
  provider: string;
}

export type BridgeStepStatus = "pending" | "in_progress" | "completed" | "failed";

export interface BridgeStep {
  label: string;
  status: BridgeStepStatus;
  txHash?: string;
  timestamp?: string;
  explorerUrl?: string;
}

export interface BridgeTransfer {
  bridgeTxId: string;
  fromChain: string;
  toChain: string;
  asset: string;
  amount: number;
  destinationAmount: number;
  fee: number;
  sourceAddress: string;
  destinationAddress: string;
  sourceTxHash?: string;
  destinationTxHash?: string;
  steps: BridgeStep[];
  currentStep: number;
  status: "pending" | "in_progress" | "completed" | "failed";
  initiatedAt: string;
  completedAt?: string;
}

export interface BridgeInitParams {
  fromChain: string;
  toChain: string;
  asset: string;
  amount: number;
  sourceAddress: string;
  destinationAddress: string;
}

export interface BridgeInitResult {
  bridgeTxId: string;
  sourceTxHash: string;
  success: boolean;
  message?: string;
}

// ── Constants ──

export const SUPPORTED_CHAINS: Chain[] = [
  {
    id: "evaporchain",
    name: "EvaporChain",
    symbol: "EVAP",
    color: "#06b6d4", // cyan
    explorerUrl: "https://explorer.evaporchain.com",
    rpcUrl: "https://testnet.evaporchain.com",
    icon: "E",
  },
  {
    id: "ethereum",
    name: "Ethereum",
    symbol: "ETH",
    color: "#627eea", // ethereum blue
    explorerUrl: "https://etherscan.io",
    rpcUrl: "https://mainnet.infura.io/v3/placeholder",
    icon: "E",
  },
  {
    id: "solana",
    name: "Solana",
    symbol: "SOL",
    color: "#9945ff", // solana purple
    explorerUrl: "https://solscan.io",
    rpcUrl: "https://api.mainnet-beta.solana.com",
    icon: "S",
  },
  {
    id: "polygon",
    name: "Polygon",
    symbol: "POL",
    color: "#8247e5", // polygon purple
    explorerUrl: "https://polygonscan.com",
    rpcUrl: "https://polygon-rpc.com",
    icon: "P",
  },
];

export const SUPPORTED_ASSETS: Asset[] = [
  { symbol: "EVAP", name: "EvaporChain", decimals: 18, nativeChain: "evaporchain" },
  { symbol: "wETH", name: "Wrapped Ether", decimals: 18, nativeChain: "ethereum" },
  { symbol: "wSOL", name: "Wrapped SOL", decimals: 9, nativeChain: "solana" },
  { symbol: "USDC", name: "USD Coin", decimals: 6, nativeChain: "ethereum" },
];

// ── Bridge Manager ──

export class BridgeManager {
  private baseUrl: string;

  constructor(baseUrl: string = BRIDGE_API) {
    this.baseUrl = baseUrl.replace(/\/+$/, "");
  }

  private async get<T>(path: string): Promise<T> {
    const res = await fetch(`${this.baseUrl}${path}`);
    if (!res.ok) throw new Error(`Bridge API ${res.status}: ${await res.text()}`);
    return res.json();
  }

  private async post<T>(path: string, body: unknown): Promise<T> {
    const res = await fetch(`${this.baseUrl}${path}`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    });
    if (!res.ok) throw new Error(`Bridge API ${res.status}: ${await res.text()}`);
    return res.json();
  }

  /**
   * Get all supported chains for bridging.
   */
  getSupportedChains(): Chain[] {
    return SUPPORTED_CHAINS;
  }

  /**
   * Get supported assets for a specific chain.
   * All assets are available on all chains as wrapped tokens.
   */
  getSupportedAssets(_chain: string): Asset[] {
    return SUPPORTED_ASSETS;
  }

  /**
   * Get a bridge quote for a specific transfer.
   * STUB: Returns calculated placeholder data.
   */
  async getQuote(
    fromChain: string,
    toChain: string,
    asset: string,
    amount: number
  ): Promise<BridgeQuote> {
    try {
      return await this.post<BridgeQuote>("/quote", {
        from_chain: fromChain,
        to_chain: toChain,
        asset,
        amount,
      });
    } catch {
      // Fallback stub quote when bridge API is unavailable
      const feePercent = 0.1;
      const fee = amount * (feePercent / 100);
      return {
        fromChain,
        toChain,
        asset,
        amount,
        fee,
        feePercent,
        estimatedTimeMinutes: 15,
        destinationAmount: amount - fee,
        exchangeRate: 1.0,
        provider: "EvaporChain Bridge v1",
      };
    }
  }

  /**
   * Initiate a bridge transfer.
   * STUB: Returns placeholder transaction data.
   */
  async initiateBridge(params: BridgeInitParams): Promise<BridgeInitResult> {
    try {
      return await this.post<BridgeInitResult>("/transfer", params);
    } catch {
      // Stub response
      const txId = `bridge_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`;
      return {
        bridgeTxId: txId,
        sourceTxHash: `0x${txId.replace("bridge_", "")}`,
        success: true,
        message: "Bridge transfer initiated (stub)",
      };
    }
  }

  /**
   * Get the current status of a bridge transfer.
   * STUB: Returns a simulated in-progress status.
   */
  async getBridgeStatus(bridgeTxId: string): Promise<BridgeTransfer> {
    try {
      return await this.get<BridgeTransfer>(`/status/${bridgeTxId}`);
    } catch {
      // Stub status
      const now = new Date().toISOString();
      return {
        bridgeTxId,
        fromChain: "evaporchain",
        toChain: "ethereum",
        asset: "EVAP",
        amount: 0,
        destinationAmount: 0,
        fee: 0,
        sourceAddress: "",
        destinationAddress: "",
        steps: [
          { label: "Initiated", status: "completed", timestamp: now },
          { label: "Source confirmed", status: "in_progress" },
          { label: "Bridge processing", status: "pending" },
          { label: "Destination confirmed", status: "pending" },
        ],
        currentStep: 1,
        status: "in_progress",
        initiatedAt: now,
      };
    }
  }

  /**
   * Get bridge history for an address.
   * STUB: Returns empty array when API is unavailable.
   */
  async getBridgeHistory(address: string): Promise<BridgeTransfer[]> {
    try {
      return await this.get<BridgeTransfer[]>(`/history/${address}`);
    } catch {
      return [];
    }
  }
}

/** Singleton bridge manager instance */
export const bridgeManager = new BridgeManager();
