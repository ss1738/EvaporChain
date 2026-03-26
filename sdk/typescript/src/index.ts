/**
 * @evaporchain/sdk — TypeScript SDK for EvaporChain
 *
 * Interact with EvaporChain nodes: query state, submit transactions,
 * manage objects, NFTs, tokens, staking, and DAO governance.
 */

export { EvaporClient } from './client';
export {
  type ChainStatus,
  type BlockInfo,
  type AccountInfo,
  type StateObject,
  type GhostRecord,
  type EventRecord,
  type TransferParams,
  type CreateObjectParams,
  type RefreshParams,
  type NftToken,
  type DeployedToken,
  type StakingPool,
  type DAOProposal,
  type FaucetResult,
} from './types';
