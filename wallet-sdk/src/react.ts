/**
 * React hooks for the EvaporChain wallet SDK.
 *
 * Optional import — only use in React projects:
 *   import { useEvaporChain, useObjects, useNfts } from "@evaporchain/wallet-sdk/react";
 *
 * Two data sources:
 * - Wallet provider (window.evaporchain): connection, signing, sending
 * - API client: chain data reads (balances, objects, transactions, staking)
 *
 * Hooks that only read data use the API client and don't require a connected
 * wallet. Hooks that sign or send require wallet connection.
 */

import { useState, useEffect, useCallback, useRef, useMemo } from "react";
import { EvaporChainProvider } from "./provider";
import { EvaporChainAPI, type ApiClientOptions } from "./api";
import type {
  Balance,
  EvaporObject,
  Nft,
  ChainStatus,
  ConnectResult,
  Transaction,
  StakingInfo,
  Validator,
  SwapQuote,
  TxResult,
  EnergyPool,
  MortalMessage,
  NftCollection,
  ContractInfo,
  ScriptInfo,
  DeployContractParams,
  CallContractParams,
  DeployScriptParams,
  CallScriptParams,
  ScriptAbi,
  ContractEvent,
} from "./types";

// ── Singletons ──

let _sharedProvider: EvaporChainProvider | null = null;
let _sharedApi: EvaporChainAPI | null = null;

function getProvider(): EvaporChainProvider {
  if (!_sharedProvider) {
    _sharedProvider = new EvaporChainProvider();
  }
  return _sharedProvider;
}

function getApi(options?: ApiClientOptions): EvaporChainAPI {
  if (!_sharedApi) {
    _sharedApi = new EvaporChainAPI(options);
  }
  return _sharedApi;
}

/** Configure the shared API client (call once at app startup). */
export function configureApi(options: ApiClientOptions): void {
  _sharedApi = new EvaporChainAPI(options);
}

// ── useEvaporChain ──

export interface UseEvaporChainResult {
  provider: EvaporChainProvider;
  api: EvaporChainAPI;
  address: string | null;
  balance: number | null;
  nonce: number | null;
  connected: boolean;
  connecting: boolean;
  error: string | null;
  connect: () => Promise<ConnectResult | undefined>;
  disconnect: () => Promise<void>;
}

/**
 * Core hook for connecting to the EvaporChain wallet.
 * Provides both the wallet provider and API client.
 *
 * @example
 * ```tsx
 * function App() {
 *   const { address, balance, connected, connect, disconnect, api } = useEvaporChain();
 *
 *   if (!connected) {
 *     return <button onClick={connect}>Connect Wallet</button>;
 *   }
 *
 *   return (
 *     <div>
 *       <p>Address: {address}</p>
 *       <p>Balance: {balance} EVAP</p>
 *       <button onClick={disconnect}>Disconnect</button>
 *     </div>
 *   );
 * }
 * ```
 */
export function useEvaporChain(): UseEvaporChainResult {
  const provider = getProvider();
  const api = getApi();
  const [address, setAddress] = useState<string | null>(provider.address);
  const [balance, setBalance] = useState<number | null>(null);
  const [nonce, setNonce] = useState<number | null>(null);
  const [connected, setConnected] = useState(provider.connected);
  const [connecting, setConnecting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const onConnect = (result: unknown) => {
      const r = result as ConnectResult;
      setAddress(r.address);
      setConnected(true);
      setError(null);
    };

    const onDisconnect = () => {
      setAddress(null);
      setBalance(null);
      setNonce(null);
      setConnected(false);
    };

    const onAccountsChanged = (accounts: unknown) => {
      const accs = accounts as string[];
      if (accs.length > 0) {
        setAddress(accs[0]);
      } else {
        setAddress(null);
        setConnected(false);
      }
    };

    provider.on("connect", onConnect);
    provider.on("disconnect", onDisconnect);
    provider.on("accountsChanged", onAccountsChanged);

    return () => {
      provider.off("connect", onConnect);
      provider.off("disconnect", onDisconnect);
      provider.off("accountsChanged", onAccountsChanged);
    };
  }, [provider]);

  // Fetch balance via API when address changes
  useEffect(() => {
    if (!address) {
      setBalance(null);
      setNonce(null);
      return;
    }

    let cancelled = false;
    api.getBalance(address).then((b: Balance) => {
      if (!cancelled) {
        setBalance(b.balance);
        setNonce(b.nonce);
      }
    }).catch(() => {});

    return () => { cancelled = true; };
  }, [address, api]);

  const connect = useCallback(async () => {
    setConnecting(true);
    setError(null);
    try {
      return await provider.connect();
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : String(err);
      setError(message);
      return undefined;
    } finally {
      setConnecting(false);
    }
  }, [provider]);

  const disconnect = useCallback(async () => {
    await provider.disconnect();
  }, [provider]);

  return { provider, api, address, balance, nonce, connected, connecting, error, connect, disconnect };
}

// ── useObjects ──

export interface UseObjectsResult {
  objects: EvaporObject[];
  loading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
}

/**
 * Fetch decaying state objects owned by an address.
 * Uses the API client — no wallet connection required for reading.
 */
export function useObjects(address?: string): UseObjectsResult {
  const api = getApi();
  const [objects, setObjects] = useState<EvaporObject[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const mountedRef = useRef(true);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await api.getObjects(address);
      if (mountedRef.current) setObjects(result);
    } catch (err: unknown) {
      if (mountedRef.current) {
        setError(err instanceof Error ? err.message : String(err));
      }
    } finally {
      if (mountedRef.current) setLoading(false);
    }
  }, [api, address]);

  useEffect(() => {
    mountedRef.current = true;
    if (address) refresh();
    return () => { mountedRef.current = false; };
  }, [address, refresh]);

  return { objects, loading, error, refresh };
}

// ── useNfts ──

export interface UseNftsResult {
  nfts: Nft[];
  loading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
}

/**
 * Fetch NFTs owned by an address.
 * Uses the API client — no wallet connection required for reading.
 */
export function useNfts(address?: string): UseNftsResult {
  const api = getApi();
  const [nfts, setNfts] = useState<Nft[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const mountedRef = useRef(true);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await api.getNFTs(address);
      if (mountedRef.current) setNfts(result);
    } catch (err: unknown) {
      if (mountedRef.current) {
        setError(err instanceof Error ? err.message : String(err));
      }
    } finally {
      if (mountedRef.current) setLoading(false);
    }
  }, [api, address]);

  useEffect(() => {
    mountedRef.current = true;
    if (address) refresh();
    return () => { mountedRef.current = false; };
  }, [address, refresh]);

  return { nfts, loading, error, refresh };
}

// ── useChainStatus ──

export interface UseChainStatusResult {
  status: ChainStatus | null;
  blockHeight: number | null;
  epoch: number | null;
  loading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
}

/**
 * Fetch current chain status. Auto-polls every 10 seconds.
 * Uses the API client — no wallet connection required.
 */
export function useChainStatus(): UseChainStatusResult {
  const api = getApi();
  const [status, setStatus] = useState<ChainStatus | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const mountedRef = useRef(true);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await api.getChainStatus();
      if (mountedRef.current) setStatus(result);
    } catch (err: unknown) {
      if (mountedRef.current) {
        setError(err instanceof Error ? err.message : String(err));
      }
    } finally {
      if (mountedRef.current) setLoading(false);
    }
  }, [api]);

  useEffect(() => {
    mountedRef.current = true;
    refresh();
    const interval = setInterval(refresh, 10_000);
    return () => {
      mountedRef.current = false;
      clearInterval(interval);
    };
  }, [refresh]);

  return {
    status,
    blockHeight: status?.blockHeight ?? null,
    epoch: status?.epoch ?? null,
    loading,
    error,
    refresh,
  };
}

// ── useTransactions ──

export interface UseTransactionsResult {
  transactions: Transaction[];
  loading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
}

/**
 * Fetch transaction history for an address.
 *
 * @param address - Address to query
 * @param limit - Max transactions to return (default: 20)
 */
export function useTransactions(address?: string, limit: number = 20): UseTransactionsResult {
  const api = getApi();
  const [transactions, setTransactions] = useState<Transaction[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const mountedRef = useRef(true);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await api.getTransactions(address, limit);
      if (mountedRef.current) setTransactions(result);
    } catch (err: unknown) {
      if (mountedRef.current) {
        setError(err instanceof Error ? err.message : String(err));
      }
    } finally {
      if (mountedRef.current) setLoading(false);
    }
  }, [api, address, limit]);

  useEffect(() => {
    mountedRef.current = true;
    if (address) refresh();
    return () => { mountedRef.current = false; };
  }, [address, refresh]);

  return { transactions, loading, error, refresh };
}

// ── useStaking ──

export interface UseStakingResult {
  info: StakingInfo | null;
  validators: Validator[];
  loading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
  stake: (amount: number) => Promise<TxResult>;
  unstake: (amount: number) => Promise<TxResult>;
  claimRewards: () => Promise<TxResult>;
}

/**
 * Staking hook — reads staking info and validators,
 * provides stake/unstake/claim actions.
 *
 * @param address - Address to query staking info for
 */
export function useStaking(address?: string): UseStakingResult {
  const api = getApi();
  const [info, setInfo] = useState<StakingInfo | null>(null);
  const [validators, setValidators] = useState<Validator[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const mountedRef = useRef(true);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [stakingInfo, validatorList] = await Promise.allSettled([
        address ? api.getStakingInfo(address) : Promise.resolve(null),
        api.getValidators(),
      ]);
      if (mountedRef.current) {
        if (stakingInfo.status === "fulfilled" && stakingInfo.value) setInfo(stakingInfo.value);
        if (validatorList.status === "fulfilled") setValidators(validatorList.value);
      }
    } catch (err: unknown) {
      if (mountedRef.current) {
        setError(err instanceof Error ? err.message : String(err));
      }
    } finally {
      if (mountedRef.current) setLoading(false);
    }
  }, [api, address]);

  useEffect(() => {
    mountedRef.current = true;
    refresh();
    return () => { mountedRef.current = false; };
  }, [refresh]);

  const stakeAction = useCallback(async (amount: number): Promise<TxResult> => {
    if (!address) throw new Error("No address connected");
    const nonce = info?.epoch ?? 0;
    const result = await api.stake(address, amount, nonce);
    refresh();
    return result;
  }, [api, address, info, refresh]);

  const unstakeAction = useCallback(async (amount: number): Promise<TxResult> => {
    if (!address) throw new Error("No address connected");
    const nonce = info?.epoch ?? 0;
    const result = await api.unstake(address, amount, nonce);
    refresh();
    return result;
  }, [api, address, info, refresh]);

  const claimRewardsAction = useCallback(async (): Promise<TxResult> => {
    if (!address) throw new Error("No address connected");
    const nonce = info?.epoch ?? 0;
    const result = await api.claimRewards(address, nonce);
    refresh();
    return result;
  }, [api, address, info, refresh]);

  return {
    info,
    validators,
    loading,
    error,
    refresh,
    stake: stakeAction,
    unstake: unstakeAction,
    claimRewards: claimRewardsAction,
  };
}

// ── useSwap ──

export interface UseSwapResult {
  quote: SwapQuote | null;
  loading: boolean;
  error: string | null;
  getQuote: (fromToken: string, toToken: string, amount: number) => Promise<void>;
  execute: (fromToken: string, toToken: string, amount: number, slippage: number) => Promise<TxResult>;
}

/**
 * Token swap hook — get quotes and execute swaps.
 */
export function useSwap(): UseSwapResult {
  const api = getApi();
  const [quote, setQuote] = useState<SwapQuote | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const getQuote = useCallback(async (fromToken: string, toToken: string, amount: number) => {
    setLoading(true);
    setError(null);
    try {
      const result = await api.getSwapQuote(fromToken, toToken, amount);
      setQuote(result);
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }, [api]);

  const execute = useCallback(async (fromToken: string, toToken: string, amount: number, slippage: number): Promise<TxResult> => {
    return api.executeSwap(fromToken, toToken, amount, slippage);
  }, [api]);

  return { quote, loading, error, getQuote, execute };
}

// ── usePools ──

export interface UsePoolsResult {
  pools: EnergyPool[];
  loading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
}

/**
 * Fetch energy pools.
 */
export function usePools(): UsePoolsResult {
  const api = getApi();
  const [pools, setPools] = useState<EnergyPool[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const mountedRef = useRef(true);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await api.getPools();
      if (mountedRef.current) setPools(result);
    } catch (err: unknown) {
      if (mountedRef.current) {
        setError(err instanceof Error ? err.message : String(err));
      }
    } finally {
      if (mountedRef.current) setLoading(false);
    }
  }, [api]);

  useEffect(() => {
    mountedRef.current = true;
    refresh();
    return () => { mountedRef.current = false; };
  }, [refresh]);

  return { pools, loading, error, refresh };
}

// ── useMessages ──

export interface UseMessagesResult {
  inbox: MortalMessage[];
  sent: MortalMessage[];
  loading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
}

/**
 * Fetch mortal messages for an address.
 */
export function useMessages(address?: string): UseMessagesResult {
  const api = getApi();
  const [inbox, setInbox] = useState<MortalMessage[]>([]);
  const [sent, setSent] = useState<MortalMessage[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const mountedRef = useRef(true);

  const refresh = useCallback(async () => {
    if (!address) return;
    setLoading(true);
    setError(null);
    try {
      const [inboxResult, sentResult] = await Promise.allSettled([
        api.getInbox(address),
        api.getSentMessages(address),
      ]);
      if (mountedRef.current) {
        if (inboxResult.status === "fulfilled") setInbox(inboxResult.value);
        if (sentResult.status === "fulfilled") setSent(sentResult.value);
      }
    } catch (err: unknown) {
      if (mountedRef.current) {
        setError(err instanceof Error ? err.message : String(err));
      }
    } finally {
      if (mountedRef.current) setLoading(false);
    }
  }, [api, address]);

  useEffect(() => {
    mountedRef.current = true;
    if (address) refresh();
    return () => { mountedRef.current = false; };
  }, [address, refresh]);

  return { inbox, sent, loading, error, refresh };
}

// ── useCollections ──

export interface UseCollectionsResult {
  collections: NftCollection[];
  loading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
}

/**
 * Fetch NFT collections.
 */
export function useCollections(): UseCollectionsResult {
  const api = getApi();
  const [collections, setCollections] = useState<NftCollection[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const mountedRef = useRef(true);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await api.getCollections();
      if (mountedRef.current) setCollections(result);
    } catch (err: unknown) {
      if (mountedRef.current) {
        setError(err instanceof Error ? err.message : String(err));
      }
    } finally {
      if (mountedRef.current) setLoading(false);
    }
  }, [api]);

  useEffect(() => {
    mountedRef.current = true;
    refresh();
    return () => { mountedRef.current = false; };
  }, [refresh]);

  return { collections, loading, error, refresh };
}

// ── useContracts ──

export interface UseContractsResult {
  contracts: ContractInfo[];
  loading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
  deploy: (params: DeployContractParams) => Promise<TxResult>;
  call: (params: CallContractParams) => Promise<TxResult>;
}

/**
 * Fetch deployed contracts and provide deploy/call actions.
 * Deploy and call require a connected wallet.
 */
export function useContracts(): UseContractsResult {
  const provider = getProvider();
  const api = getApi();
  const [contracts, setContracts] = useState<ContractInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const mountedRef = useRef(true);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await api.getContracts();
      if (mountedRef.current) setContracts(result.contracts);
    } catch (err: unknown) {
      if (mountedRef.current) {
        setError(err instanceof Error ? err.message : String(err));
      }
    } finally {
      if (mountedRef.current) setLoading(false);
    }
  }, [api]);

  useEffect(() => {
    mountedRef.current = true;
    refresh();
    return () => { mountedRef.current = false; };
  }, [refresh]);

  const deploy = useCallback(async (params: DeployContractParams): Promise<TxResult> => {
    const result = await provider.deployContract(params);
    await refresh();
    return result;
  }, [provider, refresh]);

  const call = useCallback(async (params: CallContractParams): Promise<TxResult> => {
    return provider.callContract(params);
  }, [provider]);

  return { contracts, loading, error, refresh, deploy, call };
}

// ── useScripts ──

export interface UseScriptsResult {
  scripts: ScriptInfo[];
  loading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
  deploy: (params: DeployScriptParams) => Promise<TxResult>;
  call: (params: CallScriptParams) => Promise<TxResult>;
  getAbi: (scriptId: number) => Promise<ScriptAbi>;
}

/**
 * Fetch deployed EvaporScript programs and provide deploy/call actions.
 * Deploy and call require a connected wallet.
 */
export function useScripts(): UseScriptsResult {
  const provider = getProvider();
  const api = getApi();
  const [scripts, setScripts] = useState<ScriptInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const mountedRef = useRef(true);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await api.getScripts();
      if (mountedRef.current) setScripts(result.scripts);
    } catch (err: unknown) {
      if (mountedRef.current) {
        setError(err instanceof Error ? err.message : String(err));
      }
    } finally {
      if (mountedRef.current) setLoading(false);
    }
  }, [api]);

  useEffect(() => {
    mountedRef.current = true;
    refresh();
    return () => { mountedRef.current = false; };
  }, [refresh]);

  const deploy = useCallback(async (params: DeployScriptParams): Promise<TxResult> => {
    const result = await provider.deployScript(params);
    await refresh();
    return result;
  }, [provider, refresh]);

  const call = useCallback(async (params: CallScriptParams): Promise<TxResult> => {
    return provider.callScript(params);
  }, [provider]);

  const getAbi = useCallback(async (scriptId: number): Promise<ScriptAbi> => {
    return api.getScriptAbi(scriptId);
  }, [api]);

  return { scripts, loading, error, refresh, deploy, call, getAbi };
}

// ── useContractEvents ──

export interface UseContractEventsResult {
  events: ContractEvent[];
  loading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
}

/**
 * Fetch event logs for a specific contract.
 */
export function useContractEvents(contractId?: number): UseContractEventsResult {
  const api = getApi();
  const [events, setEvents] = useState<ContractEvent[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const mountedRef = useRef(true);

  const refresh = useCallback(async () => {
    if (contractId === undefined) return;
    setLoading(true);
    setError(null);
    try {
      const result = await api.getContractEvents(contractId);
      if (mountedRef.current) setEvents(result);
    } catch (err: unknown) {
      if (mountedRef.current) {
        setError(err instanceof Error ? err.message : String(err));
      }
    } finally {
      if (mountedRef.current) setLoading(false);
    }
  }, [api, contractId]);

  useEffect(() => {
    mountedRef.current = true;
    if (contractId !== undefined) refresh();
    return () => { mountedRef.current = false; };
  }, [contractId, refresh]);

  return { events, loading, error, refresh };
}
