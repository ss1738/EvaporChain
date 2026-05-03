# EvaporChain TypeScript SDK

`@evaporchain/sdk` is a lightweight TypeScript client for the EvaporChain HTTP API. It wraps every endpoint into typed methods and adds decay-aware utilities that no other blockchain SDK has.

## Installation

```bash
npm install @evaporchain/sdk
```

## Quick Start

```typescript
import { EvaporChain } from "@evaporchain/sdk";

const chain = new EvaporChain("https://testnet.evaporchain.com");

const status = await chain.getStatus();
console.log(`Chain: ${status.chain_name} v${status.version}`);
console.log(`Block: ${status.block_height}, Epoch: ${status.epoch}`);
console.log(`Active objects: ${status.active_objects}, Ghosts: ${status.ghost_count}`);
```

## Constructor

```typescript
// Simple: pass URL string
const chain = new EvaporChain("https://testnet.evaporchain.com");

// With options
const chain = new EvaporChain({
  baseUrl: "https://testnet.evaporchain.com",
  timeout: 15000, // ms, default 10000
});

// Default: connects to https://testnet.evaporchain.com
const chain = new EvaporChain();
```

## Chain Status

```typescript
const status = await chain.getStatus();
```

Returns `ChainStatus`:

```typescript
interface ChainStatus {
  chain_name: string;       // "EvaporChain"
  version: string;          // "0.1.0"
  block_height: number;
  epoch: number;
  active_objects: number;
  ghost_count: number;
  total_evaporated: number;
  peer_count: number;
  state_root: string;       // hex
  proving_enabled: boolean;
  uptime_seconds: number;
}
```

## Accounts

```typescript
// List all accounts (sorted by balance descending)
const accounts = await chain.getAccounts();
// Account: { address, name, balance, nonce }
```

## State Objects

```typescript
// List all objects (active, grace, risen)
const objects = await chain.getObjects();

// Get single object by 64-char hex ID
const obj = await chain.getObject("0a00000000000000000000000000000000000000000000000000000000000000");
```

Returns `StateObject`:

```typescript
interface StateObject {
  id: string;
  name: string;
  owner: string;
  owner_name: string;
  energy: number;           // initial energy
  max_energy: number;
  half_life: number;        // epochs
  state: "Active" | "Grace" | "Ghost" | "Risen";
  created_epoch: number;
  last_refreshed: number;
  grace_epoch: number | null;
  current_energy: number;   // decayed energy at current epoch
  decay_percentage: number; // 0-100
}
```

## Ghost Records

```typescript
const ghosts = await chain.getGhosts();
// Ghost: { id, original_owner, evaporated_epoch, data_hash }
```

## Blocks

```typescript
// Recent blocks (most recent first)
const blocks = await chain.getBlocks(10);

// Specific block by height
const block = await chain.getBlock(42);
```

Returns `Block`:

```typescript
interface Block {
  number: number;
  epoch: number;
  parent_hash: string;
  state_root: string;
  tx_count: number;
  evaporations: number;
  entered_grace: number;
  timestamp: number;
  active_objects: number;
  ghost_count: number;
  transactions: TxRecord[];
}
```

## Events

```typescript
const events = await chain.getEvents(50);
// EventRecord: { epoch, event_type, message, timestamp_ms }
// event_type: "grace", "evaporated", "created", "refreshed", "transfer", "resurrected"
```

## Statistics

```typescript
// Aggregate stats
const summary = await chain.getStatsSummary();
// { total_created, total_evaporated, total_resurrected, total_refreshed, avg_lifetime_epochs, total_transactions }

// Epoch-by-epoch timeline
const timeline = await chain.getStatsTimeline();
// { epochs: [{ epoch, active_count, ghost_count, total_energy }] }
```

## Network

```typescript
const net = await chain.getNetwork();
// { peer_count }
```

## Transactions

### Transfer

```typescript
const result = await chain.transfer(
  1,      // from: address byte (0-255)
  2,      // to: address byte
  5000,   // amount
  0       // nonce (optional, default 0)
);
// TxResult: { success: boolean, message: string }
```

### Create Object

```typescript
const result = await chain.createObject(
  1,      // creator address byte
  42,     // object ID byte (1-255)
  10000,  // initial energy
  50      // half-life in epochs
);
```

### Refresh Object

```typescript
const result = await chain.refreshObject(
  42,     // object ID byte
  5000    // energy to deposit
);
```

### Resurrect Ghost

```typescript
const result = await chain.resurrectObject(
  42,     // object ID byte
  10000   // energy for resurrection
);
```

## Contracts

### List Contracts

```typescript
const contracts = await chain.getContracts();
// Contract: { id, template, creator, energy, half_life, created_epoch, evaporated }
```

### Get Contract

```typescript
const contract = await chain.getContract(1);
```

### Deploy Contract

```typescript
const result = await chain.deployContract(
  1,                        // deployer address byte
  "DecayingToken",          // template name
  { name: "MyToken", symbol: "MTK", supply: 1000000 }, // init args
  50000,                    // energy
  500                       // half-life
);
```

Available templates: `DecayingToken`, `MortalNFT`, `ThermodynamicEscrow`, `DecayingAuction`, `StakingPool`, `DAOVote`.

### Call Contract Method

```typescript
const result = await chain.callContract(
  1,              // caller address byte
  1,              // contract ID
  "transfer",     // method name
  { to: 2, amount: 100 },  // args
  42              // current epoch
);
```

## Faucet

```typescript
const result = await chain.claimFaucet(42);
// FaucetResult: { success: boolean, balance: number, message?: string }
```

Rate limited to once per address per hour. Grants 10,000 EVAP.

## Decay-Aware Utilities

These methods are unique to EvaporChain and don't exist in any other blockchain SDK.

### watchObject

Poll an object and get real-time decay updates. Returns a stop function.

```typescript
const stop = chain.watchObject(
  "0a00000000000000000000000000000000000000000000000000000000000000",
  (obj) => {
    console.log(`Energy: ${obj.current_energy}/${obj.max_energy}`);
    console.log(`State: ${obj.state}, Decay: ${obj.decay_percentage}%`);

    if (obj.state === "Grace") {
      console.log("Object entering grace period — refresh to save it!");
    }
    if (obj.state === "Ghost") {
      console.log("Object evaporated.");
      stop();
    }
  },
  2000  // poll interval ms (default 2000)
);

// Stop watching manually
stop();
```

### getEnergyDecayEstimate

Predict when an object will enter grace and evaporate.

```typescript
const estimate = await chain.getEnergyDecayEstimate(
  "0a00000000000000000000000000000000000000000000000000000000000000"
);

console.log(`Current energy: ${estimate.current_energy}/${estimate.max_energy}`);
console.log(`Half-life: ${estimate.half_life} epochs`);
console.log(`Decayed: ${estimate.decay_percentage}%`);
console.log(`Estimated epochs remaining: ${estimate.estimated_epochs_remaining}`);
console.log(`Will enter grace at epoch: ${estimate.will_enter_grace_at}`);
console.log(`Will evaporate at epoch: ${estimate.will_evaporate_at}`);
```

Returns `DecayEstimate`:

```typescript
interface DecayEstimate {
  current_energy: number;
  max_energy: number;
  half_life: number;
  epochs_elapsed: number;
  decay_percentage: number;
  estimated_epochs_remaining: number;
  will_enter_grace_at: number;
  will_evaporate_at: number;
}
```

### watchEvents

Stream new chain events in real-time.

```typescript
const stop = chain.watchEvents(
  (events) => {
    for (const event of events) {
      console.log(`[Epoch ${event.epoch}] ${event.event_type}: ${event.message}`);
    }
  },
  3000  // poll interval ms (default 3000)
);

// Stop watching
stop();
```

### waitForBlock

Wait until the chain reaches a specific block height.

```typescript
const block = await chain.waitForBlock(100, 30000); // target height, timeout ms
console.log(`Block ${block.number} reached at epoch ${block.epoch}`);
```

Throws `EvaporChainError` with status 408 on timeout.

## Error Handling

All methods throw `EvaporChainError` on failure:

```typescript
import { EvaporChain, EvaporChainError } from "@evaporchain/sdk";

try {
  const obj = await chain.getObject("nonexistent");
} catch (err) {
  if (err instanceof EvaporChainError) {
    console.log(`HTTP ${err.status}: ${err.message}`);
  }
}
```

## TypeScript Types

All types are exported and can be imported directly:

```typescript
import type {
  ChainStatus,
  StateObject,
  Account,
  Block,
  Ghost,
  Contract,
  TxResult,
  FaucetResult,
  DecayEstimate,
  StatsSummary,
  StatsTimeline,
  NetworkInfo,
  EventRecord,
  ClientOptions,
} from "@evaporchain/sdk";
```

## Build from Source

```bash
cd sdk/
npm install
npm run build    # compiles to dist/
npm test         # runs 23 tests
```
