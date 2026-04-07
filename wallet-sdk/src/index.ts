/**
 * @evaporchain/wallet-sdk
 *
 * Zero-dependency SDK for integrating dApps with the EvaporChain
 * browser wallet extension. Supports post-quantum ML-DSA signatures
 * and energy-based state decay.
 *
 * @example
 * ```ts
 * import { EvaporChainProvider } from "@evaporchain/wallet-sdk";
 *
 * const provider = new EvaporChainProvider();
 * const { address } = await provider.connect();
 * const { balance } = await provider.getBalance();
 * ```
 *
 * For React hooks, import from the /react subpath:
 * ```ts
 * import { useEvaporChain } from "@evaporchain/wallet-sdk/react";
 * ```
 */

export { EvaporChainProvider } from "./provider";

export {
  EvaporChainError,
  EvaporChainErrorCode,
  type EvaporObject,
  type Nft,
  type Balance,
  type ChainStatus,
  type TransactionRequest,
  type SignMessageRequest,
  type ConnectResult,
  type TransactionResult,
  type CreateObjectParams,
  type InjectedProvider,
  type EvaporChainEvent,
} from "./types";

export {
  isEvaporChainInstalled,
  formatBalance,
  shortenAddress,
  calculateDecay,
  estimateEvaporation,
} from "./utils";
