/**
 * @evaporchain/wallet-sdk
 *
 * Complete SDK for building dApps on EvaporChain.
 * Two main components:
 *
 * 1. **Provider** — wallet connection, signing, sending transactions
 *    via the browser extension (window.evaporchain)
 *
 * 2. **API client** — typed REST client for reading chain data
 *    (balances, objects, NFTs, staking, pools, messages)
 *
 * @example
 * ```ts
 * import { EvaporChainProvider, EvaporChainAPI } from "@evaporchain/wallet-sdk";
 *
 * // Wallet connection
 * const provider = new EvaporChainProvider();
 * const { address } = await provider.connect();
 *
 * // Chain data
 * const api = new EvaporChainAPI();
 * const balance = await api.getBalance(address);
 * const objects = await api.getObjects(address);
 * ```
 *
 * For React hooks, import from the /react subpath:
 * ```ts
 * import { useEvaporChain, useObjects, useStaking } from "@evaporchain/wallet-sdk/react";
 * ```
 */

// ── Provider ──

export { EvaporChainProvider } from "./provider";

// ── API Client ──

export { EvaporChainAPI, type ApiClientOptions } from "./api";

// ── Types ──

export {
  EvaporChainError,
  EvaporChainErrorCode,
  type ObjectState,
  type EvaporObject,
  type Nft,
  type NftCollection,
  type MintNftParams,
  type Balance,
  type ChainStatus,
  type TransactionRequest,
  type SignMessageRequest,
  type ConnectResult,
  type TransactionResult,
  type TxResult,
  type Transaction,
  type CreateObjectParams,
  type ContractInfo,
  type ScriptInfo,
  type DeployContractParams,
  type CallContractParams,
  type DeployScriptParams,
  type CallScriptParams,
  type AbiMethod,
  type ScriptAbi,
  type ContractEvent,
  type StakingInfo,
  type Validator,
  type SwapQuote,
  type EnergyPool,
  type PoolContribution,
  type MortalMessage,
  type InjectedProvider,
  type EvaporChainEvent,
  type NetworkId,
  type NetworkConfig,
  type PatronageStatus,
  type PatronageImmunity,
  type PatronagePledgeParams,
  type PatronagePledgeResult,
  type PatronageActionParams,
  type PatronageHonourResult,
  type PatronageRevokeResult,
  type AttractorSpec,
  type ForkChoiceModeStatus,
  type ForkChoiceAmendParams,
  type RefreshPoolCredit,
  type RefreshPoolStatus,
  type FeeControllerStatus,
  type FeeControllerStepParams,
  type FeeControllerStepResult,
  type DemurrageOwedParams,
  type DemurrageOwedResult,
  type SettleDemurrageParams,
  type SettleDemurrageResult,
  type HlwaEffectiveSupplyParams,
  type DsnStatus,
  type LadMode,
  type LadAction,
  type LadSimulateParams,
  type LadSimulateResult,
  type BellBeaconLatest,
  type BellBeaconParams,
  type BellBeaconResult,
} from "./types";

// ── Offline signing ──

export { OfflineSigner } from "./offline";

// ── Session management ──

export { SessionManager } from "./session";

// ── Offline + session types ──

export type {
  OfflineTxType,
  OfflineTransferParams,
  OfflineCreateObjectParams,
  OfflineRefreshParams,
  OfflineDeployContractParams,
  OfflineCallContractParams,
  SignableTransaction,
  SignedTransaction,
  SessionOptions,
  SessionInfo,
} from "./types";

// ── Utilities ──

export {
  isEvaporChainInstalled,
  formatBalance,
  shortenAddress,
  calculateDecay,
  estimateEvaporation,
} from "./utils";
