# EvaporChain SDK

TypeScript SDK for interacting with the EvaporChain network — the blockchain that gets lighter over time.

## Install

```bash
npm install @evaporchain/sdk
```

## Quick Start

```ts
import { EvaporChain } from "@evaporchain/sdk";

const chain = new EvaporChain("https://testnet.evaporchain.com");

// Get chain status
const status = await chain.getStatus();
console.log("Block height:", status.block_height);
console.log("Active objects:", status.active_objects);
console.log("Total evaporated:", status.total_evaporated);
```

## Watch Objects Decay in Real-Time

This is unique to EvaporChain — no other blockchain SDK has this.

```ts
const stop = chain.watchObject("1000000000000000000000000000000000000000000000000000000000000000", (obj) => {
  console.log(`${obj.name}: ${obj.current_energy}/${obj.max_energy} energy (${obj.decay_percentage}% decayed)`);

  if (obj.state === "Ghost") {
    console.log("Object evaporated!");
    stop();
  }
});
```

## Energy Decay Estimates

Predict when an object will enter grace period and evaporate:

```ts
const estimate = await chain.getEnergyDecayEstimate("1000000000000000000000000000000000000000000000000000000000000000");
console.log(`Epochs remaining: ${estimate.estimated_epochs_remaining}`);
console.log(`Will enter grace at epoch: ${estimate.will_enter_grace_at}`);
console.log(`Will evaporate at epoch: ${estimate.will_evaporate_at}`);
```

## Transactions

```ts
// Transfer tokens
await chain.transfer(0x7f, 0x2b, 1000);

// Create a decaying object (energy=5000, half-life=10 epochs)
await chain.createObject(0x7f, 0x42, 5000, 10);

// Refresh an object to prevent evaporation
await chain.refreshObject(0x42, 500);

// Resurrect a ghost
await chain.resurrectObject(0x17, 1000);

// Claim testnet tokens
await chain.claimFaucet(0xFF);
```

## Smart Contracts

```ts
// Deploy a decaying token contract
await chain.deployContract(0x7f, "DecayingToken", {
  name: "TestCoin",
  symbol: "TC",
  total_supply: 10000,
  decay_half_life: 10,
  owner: "alice",
}, 5000, 100);

// Call a contract method
await chain.callContract(0x7f, 1, "transfer", { to: "0x2b", amount: 100 }, 42);

// List contracts
const contracts = await chain.getContracts();
```

## Query Chain State

```ts
// Accounts
const accounts = await chain.getAccounts();

// Objects (active, grace, risen)
const objects = await chain.getObjects();

// Ghosts (evaporated objects)
const ghosts = await chain.getGhosts();

// Blocks
const blocks = await chain.getBlocks(10);
const block = await chain.getBlock(42);

// Events
const events = await chain.getEvents(50);

// Stats
const summary = await chain.getStatsSummary();
const timeline = await chain.getStatsTimeline();
```

## Wait for Blocks

```ts
// Wait for block 100 (30s timeout)
const block = await chain.waitForBlock(100);
```

## Watch Events

```ts
const stop = chain.watchEvents((events) => {
  for (const e of events) {
    console.log(`[${e.event_type}] ${e.message}`);
  }
});

// Stop watching later
stop();
```

## Error Handling

```ts
import { EvaporChainError } from "@evaporchain/sdk";

try {
  await chain.getObject("nonexistent");
} catch (err) {
  if (err instanceof EvaporChainError) {
    console.log(`HTTP ${err.status}: ${err.message}`);
  }
}
```

## Configuration

```ts
// String URL
const chain = new EvaporChain("https://testnet.evaporchain.com");

// Options object
const chain = new EvaporChain({
  baseUrl: "http://localhost:3000",
  timeout: 5000,
});

// Default (testnet)
const chain = new EvaporChain();
```

## API Reference

| Method | Description |
|--------|-------------|
| `getStatus()` | Chain status, block height, object counts |
| `getAccounts()` | All accounts sorted by balance |
| `getObjects()` | All state objects with energy/decay info |
| `getObject(id)` | Single object by hex ID |
| `getGhosts()` | All evaporated objects |
| `getBlocks(limit?)` | Recent blocks |
| `getBlock(height)` | Block by height |
| `getEvents(limit?)` | Recent chain events |
| `transfer(from, to, amount)` | Transfer tokens |
| `createObject(creator, id, energy, halfLife)` | Create decaying object |
| `refreshObject(id, energy)` | Refresh object energy |
| `resurrectObject(id, energy)` | Resurrect a ghost |
| `claimFaucet(address)` | Claim testnet tokens |
| `getContracts()` | List deployed contracts |
| `deployContract(...)` | Deploy smart contract |
| `callContract(...)` | Call contract method |
| `waitForBlock(height, timeout?)` | Wait for block height |
| `watchObject(id, callback, interval?)` | Watch object decay |
| `watchEvents(callback, interval?)` | Watch chain events |
| `getEnergyDecayEstimate(id)` | Predict evaporation time |
| `getStatsSummary()` | Aggregate chain stats |
| `getStatsTimeline()` | Epoch-by-epoch timeline |
| `getNetwork()` | Network peer info |

## License

MIT
