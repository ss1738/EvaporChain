# Smart Contract Templates

EvaporChain supports two ways to deploy smart contracts:

1. **Template contracts** — pre-built patterns you deploy with configuration arguments
2. **EvaporScript contracts** — custom contracts written in [EvaporScript](evaporscript.md)

Both types have thermodynamic decay: contracts have energy and a half-life. When energy reaches zero, the contract evaporates.

## Template Contracts

Templates are battle-tested contract patterns compiled into the node. You deploy them by name with initialization arguments — no code to write.

### DecayingToken

A fungible token whose total supply decays over time.

```bash
curl -X POST https://testnet.evaporchain.com/api/tx/deploy-contract \
  -H "Content-Type: application/json" \
  -d '{
    "deployer": 1,
    "template": "DecayingToken",
    "init_args": {
      "name": "Flux Token",
      "symbol": "FLUX",
      "supply": 200000
    },
    "energy": 50000,
    "half_life": 200
  }'
```

**Methods:**
- `transfer(to, amount)` — transfer tokens
- `balance(address)` — check balance
- `total_supply()` — get current supply (after decay)

**Use cases:** loyalty points, seasonal currencies, time-limited incentive tokens.

### MortalNFT

An NFT that lives and dies. Energy decay creates urgency — refresh to keep it alive or let it become a ghost.

```bash
curl -X POST https://testnet.evaporchain.com/api/tx/deploy-contract \
  -H "Content-Type: application/json" \
  -d '{
    "deployer": 1,
    "template": "MortalNFT",
    "init_args": {
      "name": "Ephemeral Art #1",
      "metadata_uri": "ipfs://Qm..."
    },
    "energy": 10000,
    "half_life": 100
  }'
```

**Methods:**
- `transfer(to)` — transfer ownership
- `metadata()` — get metadata URI
- `time_remaining()` — estimated epochs until evaporation

**Use cases:** expiring event tickets, temporary access passes, art that ages.

### ThermodynamicEscrow

An escrow that releases funds when conditions are met — or evaporates if they're not met in time. No need for dispute resolution: the thermodynamics handle it.

```bash
curl -X POST https://testnet.evaporchain.com/api/tx/deploy-contract \
  -H "Content-Type: application/json" \
  -d '{
    "deployer": 1,
    "template": "ThermodynamicEscrow",
    "init_args": {
      "buyer": 2,
      "seller": 3,
      "amount": 10000
    },
    "energy": 5000,
    "half_life": 50
  }'
```

**Methods:**
- `confirm()` — buyer confirms, releasing funds to seller
- `dispute()` — flag a dispute
- `status()` — check escrow state

**Behavior:** if nobody confirms before the contract evaporates, funds return to the buyer. The energy half-life determines the dispute window.

**Use cases:** peer-to-peer trades, freelance payments, trustless commerce.

### DecayingAuction

An auction with a natural end. No need for an explicit closing time — the auction evaporates when energy runs out.

```bash
curl -X POST https://testnet.evaporchain.com/api/tx/deploy-contract \
  -H "Content-Type: application/json" \
  -d '{
    "deployer": 1,
    "template": "DecayingAuction",
    "init_args": {
      "item_name": "Rare Artifact",
      "min_bid": 100
    },
    "energy": 10000,
    "half_life": 30
  }'
```

**Methods:**
- `bid(amount)` — place a bid (must exceed current highest)
- `current_bid()` — get highest bid
- `bidder_count()` — number of unique bidders

**Behavior:** when the contract evaporates, the highest bidder wins. Low energy creates urgency. Refreshing the contract extends the auction.

**Use cases:** NFT auctions, Dutch auctions, resource allocation.

### StakingPool

A staking pool where rewards decay if not claimed. Creates urgency to actively participate rather than passively accumulate.

```bash
curl -X POST https://testnet.evaporchain.com/api/tx/deploy-contract \
  -H "Content-Type: application/json" \
  -d '{
    "deployer": 1,
    "template": "StakingPool",
    "init_args": {
      "reward_rate": 100,
      "reward_decay_hl": 50
    },
    "energy": 100000,
    "half_life": 1000
  }'
```

**Methods:**
- `stake(amount)` — stake tokens
- `unstake(amount)` — unstake tokens
- `claim()` — claim pending rewards (before they decay)
- `pending_rewards()` — check unclaimed rewards

**Use cases:** validator incentives, liquidity mining, active participation rewards.

### DAOVote

A governance proposal that evaporates. Proposals that don't get enough attention naturally die.

```bash
curl -X POST https://testnet.evaporchain.com/api/tx/deploy-contract \
  -H "Content-Type: application/json" \
  -d '{
    "deployer": 1,
    "template": "DAOVote",
    "init_args": {
      "title": "Increase reward rate",
      "options": ["For", "Against", "Abstain"],
      "voting_period": 200
    },
    "energy": 20000,
    "half_life": 200
  }'
```

**Methods:**
- `vote(option, weight)` — cast a vote
- `tally()` — get current vote counts
- `status()` — check proposal status

**Use cases:** on-chain governance, community decisions, parameter changes.

## Choosing Energy and Half-life

The contract's `energy` and `half_life` together determine its lifespan:

```
lifespan ≈ half_life * log2(energy)
```

| Energy | Half-life | Approximate Lifespan |
|--------|-----------|---------------------|
| 1,000 | 10 | ~100 epochs |
| 10,000 | 50 | ~665 epochs |
| 100,000 | 100 | ~1,660 epochs |
| 1,000,000 | 500 | ~9,966 epochs |

Choose based on your use case:
- **Flash contracts** (minutes): low energy, low half-life
- **Session contracts** (hours): medium energy, low half-life
- **Campaign contracts** (days): high energy, medium half-life
- **Infrastructure contracts** (months): very high energy, high half-life

## Contract State After Evaporation

When a template contract evaporates:

1. `on_evaporate()` logic is executed (if defined in the template)
2. The contract's full state is removed from the chain
3. A ghost record is created with the contract's metadata hash
4. The contract ID becomes available for resurrection

## EvaporScript vs Templates

| Feature | Templates | EvaporScript |
|---------|-----------|-------------|
| Deployment | Config-only, no code | Write custom source code |
| Flexibility | Fixed logic | Custom logic |
| Gas costs | Optimized | Gas-metered bytecode |
| Auditability | Pre-audited | Custom code review |
| Lifecycle hooks | Built-in | Define your own |
| Best for | Standard patterns | Novel applications |

Use templates when a standard pattern fits. Use EvaporScript when you need custom logic. Both support the same decay mechanics.

## SDK Integration

```typescript
import { EvaporChain } from "@evaporchain/sdk";

const chain = new EvaporChain("https://testnet.evaporchain.com");

// Deploy a template contract
await chain.deployContract(
  1,                    // deployer
  "DecayingToken",      // template
  { name: "MyToken", symbol: "MTK", supply: 500000 },
  50000,                // energy
  500                   // half-life
);

// List all contracts
const contracts = await chain.getContracts();

// Call a method
await chain.callContract(1, 1, "transfer", { to: 2, amount: 1000 }, 42);

// Check contract details
const contract = await chain.getContract(1);
console.log(`Energy: ${contract.energy}, Evaporated: ${contract.evaporated}`);
```
