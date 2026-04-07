# @evaporchain/wallet-sdk

Zero-dependency SDK for integrating dApps with the EvaporChain browser wallet extension. Supports post-quantum ML-DSA signatures and energy-based state decay.

## Installation

```bash
npm install @evaporchain/wallet-sdk
```

## Quick Start (5 minutes)

### 1. Connect to the wallet

```ts
import { EvaporChainProvider, isEvaporChainInstalled } from "@evaporchain/wallet-sdk";

if (!isEvaporChainInstalled()) {
  console.log("Please install the EvaporChain wallet extension");
}

const provider = new EvaporChainProvider();
const { address, publicKey } = await provider.connect();
console.log("Connected:", address);
```

### 2. Get balance

```ts
const { balance, nonce } = await provider.getBalance();
console.log("Balance:", balance, "EVAP");
```

### 3. Send a transaction

```ts
const { hash, status } = await provider.sendTransaction({
  to: "0x1a2b3c...",
  amount: 1000,
});
console.log("TX:", hash, status);
```

### 4. Refresh a decaying object

This is unique to EvaporChain — objects lose energy over time and must be refreshed to prevent evaporation.

```ts
const { hash } = await provider.refreshObject("0xobject_id...", 500);
console.log("Refreshed, tx:", hash);
```

### 5. Create a new object

```ts
const { hash, objectId } = await provider.createObject({
  name: "My Decaying NFT",
  energy: 5000,
  halfLife: 100, // epochs
});
console.log("Created object:", objectId);
```

## React Hooks

Import from the `/react` subpath:

```tsx
import { useEvaporChain, useObjects, useNfts, useChainStatus } from "@evaporchain/wallet-sdk/react";
```

### useEvaporChain

```tsx
function App() {
  const { address, balance, connected, connect, disconnect, error } = useEvaporChain();

  if (!connected) {
    return <button onClick={connect}>Connect EvaporChain Wallet</button>;
  }

  return (
    <div>
      <p>Address: {address}</p>
      <p>Balance: {balance} EVAP</p>
      <button onClick={disconnect}>Disconnect</button>
    </div>
  );
}
```

### useObjects

```tsx
function ObjectList() {
  const { objects, loading, refresh } = useObjects();

  if (loading) return <p>Loading objects...</p>;

  return (
    <ul>
      {objects.map(obj => (
        <li key={obj.id}>
          {obj.name}: {obj.currentEnergy}/{obj.maxEnergy} energy
          ({obj.state})
        </li>
      ))}
      <button onClick={refresh}>Refresh</button>
    </ul>
  );
}
```

### useNfts

```tsx
function NftGallery() {
  const { nfts, loading } = useNfts();

  if (loading) return <p>Loading NFTs...</p>;

  return nfts.map(nft => (
    <div key={nft.id}>
      <img src={nft.imageUrl} alt={nft.name} />
      <p>{nft.name} — {nft.currentEnergy} energy</p>
    </div>
  ));
}
```

### useChainStatus

```tsx
function StatusBar() {
  const { blockHeight, epoch } = useChainStatus();
  return <p>Block {blockHeight} | Epoch {epoch}</p>;
}
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

// Check if wallet is installed
isEvaporChainInstalled(); // true | false

// Format raw balance
formatBalance(1_500_000_000); // "1.5"
formatBalance(123456, 4);     // "12.3456"

// Shorten address for display
shortenAddress("0x1a2b3c4d5e6f7890abcdef1234567890abcd9f0e");
// "0x1a2b...9f0e"

// Calculate energy after decay
calculateDecay(1000, 10, 10); // 500 (one half-life)
calculateDecay(1000, 10, 20); // 250 (two half-lives)

// Estimate epochs until evaporation
estimateEvaporation(1000, 10); // ~100 epochs
```

## TypeScript Types

All types are exported from the main entry point:

```ts
import type {
  EvaporObject,     // Decaying state object
  Nft,              // NFT with energy/decay
  Balance,          // { balance, nonce }
  ChainStatus,      // { blockHeight, epoch, ... }
  TransactionRequest,
  TransactionResult,
  ConnectResult,
  CreateObjectParams,
  SignMessageRequest,
  EvaporChainEvent,
  InjectedProvider, // window.evaporchain interface
} from "@evaporchain/wallet-sdk";
```

## Error Handling

The SDK throws `EvaporChainError` with typed error codes:

```ts
import { EvaporChainError, EvaporChainErrorCode } from "@evaporchain/wallet-sdk";

try {
  await provider.connect();
} catch (err) {
  if (err instanceof EvaporChainError) {
    switch (err.code) {
      case EvaporChainErrorCode.NOT_INSTALLED:
        // Wallet extension not found
        showInstallPrompt();
        break;
      case EvaporChainErrorCode.USER_REJECTED:
        // User denied the request
        break;
      case EvaporChainErrorCode.NETWORK_ERROR:
        // RPC / network failure
        break;
      case EvaporChainErrorCode.INSUFFICIENT_BALANCE:
        // Not enough EVAP
        break;
      case EvaporChainErrorCode.OBJECT_NOT_FOUND:
        // Object doesn't exist on-chain
        break;
    }
  }
}
```

## Provider Events

```ts
provider.on("connect", (result) => {
  console.log("Connected:", result.address);
});

provider.on("disconnect", () => {
  console.log("Disconnected");
});

provider.on("accountsChanged", (accounts) => {
  console.log("Active account changed:", accounts[0]);
});

provider.on("chainChanged", (chainId) => {
  console.log("Network changed:", chainId);
});
```

## How It Works

The SDK detects the `window.evaporchain` provider injected by the EvaporChain browser extension. All signing happens inside the extension using ML-DSA post-quantum signatures — private keys never leave the wallet.

## License

MIT
