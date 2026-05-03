# EvaporScript Language Reference

EvaporScript is a non-Turing-complete scripting language for EvaporChain smart contracts. Every contract has thermodynamic decay built into the language itself — contracts have energy that depletes over time, and lifecycle hooks let you respond to evaporation events.

## Contract Structure

```
contract ContractName {
    state {
        field_name: type = default_value
    }

    fn method_name(param: type) -> return_type {
        // body
    }

    on_evaporate() {
        // called when energy reaches zero
    }
}
```

A contract has three sections:

- **state** — persistent storage fields with types and optional defaults
- **fn** — callable methods with parameters and optional return types
- **lifecycle hooks** — `on_evaporate()`, `on_grace()`, `on_refresh()` — called automatically by the chain

## Types

| Type | Description | Default | Example |
|------|-------------|---------|---------|
| `u64` | Unsigned 64-bit integer | `0` | `42`, `1000000` |
| `bool` | Boolean | `false` | `true`, `false` |
| `string` | UTF-8 string | `""` | `"hello world"` |
| `address` | 32-byte account address | `0x00...00` | — |
| `map[K -> V]` | Key-value mapping | `{}` | `map[address -> u64]` |

## Operators

### Arithmetic

| Operator | Description |
|----------|-------------|
| `+` | Addition |
| `-` | Subtraction |
| `*` | Multiplication |
| `/` | Division |

### Comparison

| Operator | Description |
|----------|-------------|
| `==` | Equal |
| `!=` | Not equal |
| `>` | Greater than |
| `<` | Less than |
| `>=` | Greater or equal |
| `<=` | Less or equal |

### Logical

| Operator | Description |
|----------|-------------|
| `&&` | Logical AND |
| `\|\|` | Logical OR |
| `!` | Logical NOT |

### Assignment

| Operator | Description |
|----------|-------------|
| `=` | Assign |
| `+=` | Add-assign |
| `-=` | Subtract-assign |

## Statements

```
// Variable declaration
let x = 42

// State field assignment
self.field = value
self.field += value

// Map operations
self.map_field[key] = value
self.map_field[key] += value

// Conditional
if condition {
    // ...
} else {
    // ...
}

// Assertion (reverts on false)
require(condition, "error message")

// Emit chain event
emit("event message")

// Return value
return expression
```

## Built-in Functions

| Function | Returns | Description |
|----------|---------|-------------|
| `caller` | `address` | Address of the transaction sender |
| `owner` | `address` | Address of the contract deployer |
| `epoch` | `u64` | Current chain epoch |
| `block_number` | `u64` | Current chain block number |
| `energy` | `u64` | Contract's remaining energy |
| `energy_of(obj_id)` | `u64` | Remaining energy of an arbitrary object |
| `balance(addr)` | `u64` | On-chain EVAP balance of an address |
| `transfer(to, amount)` | — | Transfer EVAP tokens to an address |
| `emit(msg)` | — | Emit a freeform-string contract event |
| `emit_event(name, [topics], data)` | — | Emit a structured event with topic + data |
| `require(cond, msg)` | — | Revert execution if condition is false |
| `require_epoch_range(min, max)` | — | Revert unless current epoch ∈ [min, max) |
| `compute_decay(initial, half_life, elapsed)` | `u64` | Compute decayed energy from initial value |
| `vrf_randomness()` | `u64` | Current block's VRF beacon value (truncated) |
| `vrf_domain_randomness(dom)` | `u64` | Domain-separated VRF randomness |
| `random_range(max)` | `u64` | Uniform `u64` in `[0, max)` derived from beacon |
| `call_external(contract_id, method, args…)` | `value` | Cross-contract call (gas-bounded) |

The compiler also surfaces array primitives as opcodes (`array_new`,
`array_get`, `array_set`) and map primitives (`map_get`, `map_set`),
exposed in EvaporScript via `[]` indexing on declared `array` and
`map` state fields.

Compiler `Op` enum (`crates/evaporchain-script/src/compiler.rs:9`)
has 44 opcodes total; this table covers every user-visible builtin
and primitive but groups stack/control/arithmetic ops as language
operators rather than functions.

## Lifecycle Hooks

The EvaporChain runtime calls lifecycle hooks automatically when a contract's energy state changes. These hooks are optional — only define the ones you need.

### `on_evaporate()`

Called when the contract's energy reaches zero and the contract is about to be evaporated. Use this for cleanup, final notifications, or archival.

```
on_evaporate() {
    emit("contract expired, final state archived")
}
```

### `on_grace()`

Called when the contract enters the grace period (energy depleted but not yet evaporated). The contract can still be saved by refreshing its energy.

```
on_grace() {
    emit("contract energy low, refresh to prevent evaporation")
}
```

### `on_refresh()`

Called when the contract receives an energy deposit, extending its lifetime.

```
on_refresh() {
    emit("contract energy restored")
}
```

## Gas Costs

Every operation consumes gas. If gas exceeds the limit, execution reverts.

| Operation | Cost |
|-----------|------|
| Push value | 1 |
| Load/Store variable | 2 |
| Load state field | 5 |
| Store state field | 10 |
| Arithmetic (+, -, *, /) | 3-5 |
| Comparison | 3 |
| Logic (&&, \|\|, !) | 3 |
| Jump | 2 |
| Built-in call | 10 |
| Map get | 10 |
| Map set | 20 |
| Require | 5 |
| Emit event | 8 |
| Return | 1 |

## Execution Model

EvaporScript compiles to a stack-based bytecode (EvaporBytecode) that runs on the EvaporVM:

1. **Parse** — source code is tokenized and parsed into an AST
2. **Compile** — the AST is compiled into bytecode with a method table and state schema
3. **Execute** — the VM executes bytecode against contract state with gas metering

The VM is deliberately non-Turing-complete: no unbounded loops, no recursion. This ensures predictable gas costs and prevents infinite execution.

## Example Contracts

### Counter

Minimal contract that increments a counter. Evaporates when its energy runs out.

```
contract Counter {
    state {
        count: u64 = 0
    }

    fn increment(n: u64) {
        self.count += n
    }

    fn get() -> u64 {
        return self.count
    }

    on_evaporate() {
        emit("counter expired")
    }
}
```

### Loyalty Points

A points system where a shop owner issues points to customers. The entire loyalty program expires when contract energy runs out.

```
contract LoyaltyPoints {
    state {
        name: string = "ShopPoints"
        points: map[address -> u64]
        total_issued: u64 = 0
    }

    fn issue(to: address, amount: u64) {
        require(caller == owner, "only owner")
        self.points[to] += amount
        self.total_issued += amount
    }

    fn spend(amount: u64) {
        require(self.points[caller] >= amount, "insufficient points")
        self.points[caller] -= amount
    }

    fn balance(addr: address) -> u64 {
        return self.points[addr]
    }

    on_evaporate() {
        emit("loyalty program expired")
    }
}
```

### Expiring Event Ticket

A ticket that can only be used once. The ticket system evaporates after the event window passes.

```
contract EventTicket {
    state {
        event_name: string = "Blockchain Summit 2026"
        holder: map[address -> bool]
        used: map[address -> bool]
        tickets_sold: u64 = 0
        max_tickets: u64 = 100
    }

    fn buy_ticket() {
        require(self.tickets_sold < self.max_tickets, "sold out")
        require(self.holder[caller] == false, "already has ticket")
        self.holder[caller] = true
        self.tickets_sold += 1
    }

    fn use_ticket() {
        require(self.holder[caller] == true, "no ticket")
        require(self.used[caller] == false, "already used")
        self.used[caller] = true
        emit("ticket used")
    }

    on_evaporate() {
        emit("event window closed, tickets expired")
    }

    on_grace() {
        emit("event ending soon, use your tickets!")
    }
}
```

### Decaying Auction

An auction where the entire auction evaporates if nobody bids in time.

```
contract DecayingAuction {
    state {
        item: string = "Rare NFT"
        highest_bid: u64 = 0
        highest_bidder: map[u64 -> address]
        bid_count: u64 = 0
    }

    fn bid(amount: u64) {
        require(amount > self.highest_bid, "bid too low")
        self.highest_bid = amount
        self.bid_count += 1
        self.highest_bidder[self.bid_count] = caller
        emit("new highest bid")
    }

    fn current_bid() -> u64 {
        return self.highest_bid
    }

    on_evaporate() {
        emit("auction ended")
    }

    on_grace() {
        emit("auction closing soon, last chance to bid!")
    }
}
```

## Deploy and Call

### Deploy via API

```bash
curl -X POST https://testnet.evaporchain.com/api/tx/deploy-script \
  -H "Content-Type: application/json" \
  -d '{
    "deployer": 1,
    "source_code": "contract Counter { state { count: u64 = 0 } fn increment(n: u64) { self.count += n } fn get() -> u64 { return self.count } on_evaporate() { emit(\"counter expired\") } }",
    "energy": 10000,
    "half_life": 200
  }'
```

### Call a Method

```bash
curl -X POST https://testnet.evaporchain.com/api/tx/call-script \
  -H "Content-Type: application/json" \
  -d '{
    "caller": 1,
    "contract_id": 1,
    "method": "increment",
    "args": "[{\"U64\": 42}]",
    "epoch": 10
  }'
```

### Deploy via SDK

```typescript
import { EvaporChain } from "@evaporchain/sdk";

const chain = new EvaporChain("https://testnet.evaporchain.com");

// Deploy using a template (simpler)
await chain.deployContract(1, "DecayingToken", {
  name: "MyToken",
  symbol: "MTK",
  supply: 1000000,
}, 50000, 500);

// Call a method
await chain.callContract(1, 1, "transfer", { to: 2, amount: 100 }, 42);
```

## Design Philosophy

EvaporScript is intentionally limited compared to languages like Solidity or Move:

- **No unbounded loops** — prevents gas bombs and infinite execution
- **No recursion** — eliminates reentrancy attacks by design
- **Built-in decay** — every contract has a natural lifespan, preventing abandoned contract bloat
- **Lifecycle hooks** — first-class support for responding to state transitions
- **Explicit state schema** — all storage is declared upfront, no hidden slots

The constraint is the feature: contracts that know they will die can make better decisions about their final state.
