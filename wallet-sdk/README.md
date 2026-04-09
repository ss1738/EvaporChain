# @evaporchain/wallet-sdk

Complete SDK for building dApps on EvaporChain. Provides wallet connection, a typed REST API client, React hooks, and utility functions. Zero external dependencies.

## Installation

```bash
npm install @evaporchain/wallet-sdk
```

## What's Included

| Module | Import | Purpose |
|--------|--------|---------|
| **Provider** | `@evaporchain/wallet-sdk` | Wallet connection, signing, sending transactions |
| **API Client** | `@evaporchain/wallet-sdk` | Typed REST client for reading chain data |
| **React Hooks** | `@evaporchain/wallet-sdk/react` | Hooks for wallet, objects, NFTs, staking, messages |
| **Utilities** | `@evaporchain/wallet-sdk` | Balance formatting, decay calculation, address display |

## Quick Start

### Wallet Connection + API Client

```ts
import { EvaporChainProvider, EvaporChainAPI } from "@evaporchain/wallet-sdk";

// Connect to wallet (browser extension)
const provider = new EvaporChainProvider();
const { address } = await provider.connect();

// Read chain data via API
const api = new EvaporChainAPI(); // defaults to testnet
const balance = await api.getBalance(address);
const objects = await api.getObjects(address);
const staking = await api.getStakingInfo(address);
```

### React (Recommended for dApps)

```tsx
import { useEvaporChain, useObjects, useStaking } from "@evaporchain/wallet-sdk/react";

function App() {
  const { address, balance, connected, connect, disconnect } = useEvaporChain();
  const { objects, loading } = useObjects(address ?? undefined);

  if (!connected) {
    return <button onClick={connect}>Connect Wallet</button>;
  }

  return (
    <div>
      <p>{address} — {balance} EVAP</p>
      {objects.map(obj => (
        <div key={obj.id}>{obj.name}: {obj.currentEnergy}/{obj.maxEnergy}</div>
      ))}
      <button onClick={disconnect}>Disconnect</button>
    </div>
  );
}
```

## API Client

The API client provides typed access to all EvaporChain REST endpoints. No wallet connection required for reading data.

```ts
import { EvaporChainAPI } from "@evaporchain/wallet-sdk";

const api = new EvaporChainAPI(); // testnet
// or: new EvaporChainAPI({ network: "mainnet" })
// or: new EvaporChainAPI({ rpcUrl: "http://localhost:3000" })
```

### Chain & Accounts

```ts
const status = await api.getChainStatus();     // block height, epoch, peers
const balance = await api.getBalance(address);  // balance, nonce
const txns = await api.getTransactions(address, 20); // transaction history
```

### Objects (Decaying State)

```ts
const objects = await api.getObjects(address);        // all objects
const critical = await api.getObjectsByState(address, "Grace"); // filter by state
const single = await api.getObject(objectId);         // single object
await api.refreshObject(objectId, 1000);              // deposit energy
await api.batchRefresh([                               // batch refresh
  { id: "0xabc...", energy: 500 },
  { id: "0xdef...", energy: 500 },
]);
```

### NFTs

```ts
const nfts = await api.getNFTs(address);
const collections = await api.getCollections();
await api.mintNFT({ name: "My NFT", collection: "col_id", energy: 5000, halfLife: 100 });
await api.refreshNFT(nftId, 500);
await api.transferNFT(nftId, recipientAddress);
```

### Staking

```ts
const info = await api.getStakingInfo(address);
const validators = await api.getValidators();
await api.stake(address, 10000, nonce);
await api.unstake(address, 5000, nonce);
await api.claimRewards(address, nonce);
```

### Token Swap

```ts
const quote = await api.getSwapQuote("EVAP", "wETH", 1000);
await api.executeSwap("EVAP", "wETH", 1000, 0.5); // 0.5% slippage
```

### Energy Pools

```ts
const pools = await api.getPools();
const pool = await api.getPool(poolId);
const contributors = await api.getPoolContributors(poolId);
await api.createPool("My Pool", creatorAddress);
await api.stakeToPool(poolId, address, 500);
```

### Mortal Messages

```ts
await api.sendMessage(from, to, "Hello!", 1000);
const inbox = await api.getInbox(address);
const sent = await api.getSentMessages(address);
await api.boostMessage(messageId, 500);
```

### Faucet (Testnet)

```ts
await api.claimFaucet(address);
```

## React Hooks

All hooks are available from `@evaporchain/wallet-sdk/react`:

```tsx
import {
  useEvaporChain,     // wallet connection + balance
  useObjects,         // decaying state objects
  useNfts,            // NFTs with decay
  useChainStatus,     // block height, epoch (auto-polls)
  useTransactions,    // transaction history
  useStaking,         // staking info + actions
  useSwap,            // swap quotes + execution
  usePools,           // energy pools
  useMessages,        // mortal messages
  useCollections,     // NFT collections
  configureApi,       // configure API endpoint
} from "@evaporchain/wallet-sdk/react";
```

### Hook Examples

```tsx
// Staking with actions
function StakingPanel() {
  const { address } = useEvaporChain();
  const { info, validators, stake, unstake, claimRewards } = useStaking(address);

  return (
    <div>
      <p>Staked: {info?.staked} EVAP</p>
      <p>Rewards: {info?.rewards} EVAP</p>
      <button onClick={() => stake(1000)}>Stake 1000</button>
      <button onClick={() => claimRewards()}>Claim</button>
    </div>
  );
}

// Transaction history
function History() {
  const { address } = useEvaporChain();
  const { transactions, loading } = useTransactions(address, 50);

  return transactions.map(tx => (
    <div key={tx.hash}>{tx.type}: {tx.amount} EVAP</div>
  ));
}

// Swap
function SwapWidget() {
  const { quote, getQuote, execute } = useSwap();

  return (
    <div>
      <button onClick={() => getQuote("EVAP", "wETH", 100)}>Get Quote</button>
      {quote && <p>Rate: {quote.rate}</p>}
      <button onClick={() => execute("EVAP", "wETH", 100, 0.5)}>Swap</button>
    </div>
  );
}
```

## Wallet Provider

For direct wallet interaction (signing, sending transactions):

```ts
import { EvaporChainProvider } from "@evaporchain/wallet-sdk";

const provider = new EvaporChainProvider();
const { address } = await provider.connect();

// Send transaction (opens wallet popup)
const { hash } = await provider.sendTransaction({ to: "0x...", amount: 1000 });

// Sign message (ML-DSA post-quantum signature)
const { signature } = await provider.signMessage("Hello EvaporChain");

// Refresh decaying object
await provider.refreshObject("0xobject_id", 500);

// Create new object
const { objectId } = await provider.createObject({
  name: "My Object",
  energy: 5000,
  halfLife: 100,
});
```

## Utility Functions

```ts
import {
  isEvaporChainInstalled,
  formatBalance,
  shortenAddress,
  calculateDecay,
  estimateEvaporation,
} from "@evaporchain/wallet-sdk";

isEvaporChainInstalled();          // true | false
formatBalance(1_500_000_000);      // "1.5"
shortenAddress("0x1a2b...9f0e");   // "0x1a2b...9f0e"
calculateDecay(1000, 10, 10);      // 500 (one half-life)
estimateEvaporation(1000, 10);     // ~100 epochs
```

## Error Handling

```ts
import { EvaporChainError, EvaporChainErrorCode } from "@evaporchain/wallet-sdk";

try {
  await provider.connect();
} catch (err) {
  if (err instanceof EvaporChainError) {
    switch (err.code) {
      case EvaporChainErrorCode.NOT_INSTALLED:
        showInstallPrompt();
        break;
      case EvaporChainErrorCode.USER_REJECTED:
        // User denied
        break;
      case EvaporChainErrorCode.NETWORK_ERROR:
        // RPC failure
        break;
      case EvaporChainErrorCode.INSUFFICIENT_BALANCE:
        // Not enough EVAP
        break;
      case EvaporChainErrorCode.OBJECT_NOT_FOUND:
        // Object doesn't exist
        break;
    }
  }
}
```

## Network Configuration

```ts
import { EvaporChainAPI } from "@evaporchain/wallet-sdk";

const api = new EvaporChainAPI({ network: "testnet" });  // default
api.setNetwork("mainnet");                                 // switch at runtime

// Custom RPC
const localApi = new EvaporChainAPI({ rpcUrl: "http://localhost:3000" });
```

For React hooks, configure once at app startup:

```ts
import { configureApi } from "@evaporchain/wallet-sdk/react";
configureApi({ network: "mainnet" });
```

## Migrating from Custom Hooks

If your dApp uses a custom `useWalletConnect` hook that calls `window.evaporchain` directly, replace it with the SDK:

```ts
// Before (custom hook)
import { useWalletConnect } from "./hooks/useWalletConnect";
const { address, connected, connect, disconnect } = useWalletConnect();

// After (SDK hook)
import { useEvaporChain } from "@evaporchain/wallet-sdk/react";
const { address, connected, connect, disconnect } = useEvaporChain();
```

The SDK handles provider detection, event forwarding, reconnection, and type safety — all the boilerplate that custom hooks reimplement inconsistently.

## How It Works

- **Provider**: Detects `window.evaporchain` injected by the browser extension. All signing uses ML-DSA post-quantum signatures inside the extension — private keys never leave the wallet.
- **API Client**: Makes typed HTTP requests to EvaporChain RPC nodes. Snake_case responses are automatically converted to camelCase.
- **React Hooks**: Singleton provider + API client shared across all hook instances. Auto-cleanup on unmount.

## License

MIT
