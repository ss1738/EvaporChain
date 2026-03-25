# EvaporScript Language Guide

EvaporScript is a lightweight, non-Turing-complete scripting language for EvaporChain smart contracts. Every contract has thermodynamic decay built into the language itself — contracts have energy that depletes over time, and lifecycle hooks let you respond to evaporation events.

## Syntax Overview

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

A contract has three sections:
- **state** — declares persistent storage fields with types and optional defaults
- **fn** — defines callable methods with parameters and optional return types
- **lifecycle hooks** — `on_evaporate()`, `on_grace()`, `on_refresh()` — called automatically by the chain

## Types

| Type | Description | Example |
|------|-------------|---------|
| `u64` | Unsigned 64-bit integer | `42`, `0`, `1000000` |
| `bool` | Boolean | `true`, `false` |
| `string` | UTF-8 string | `"hello world"` |
| `address` | 32-byte account address | — |
| `map[K -> V]` | Key-value mapping | `map[address -> u64]` |

## Operators

### Arithmetic
`+`, `-`, `*`, `/`

### Comparison
`==`, `!=`, `>`, `<`, `>=`, `<=`

### Logical
`&&`, `||`, `!`

### Assignment
`=`, `+=`, `-=`

## Statements

```
let x = 42                           // variable declaration
self.field = value                   // state field assignment
self.field += value                  // compound assignment
self.map_field[key] = value          // map entry assignment
self.map_field[key] += value         // compound map assignment

if condition {                       // conditional
    // ...
} else {
    // ...
}

require(condition, "error message")  // assertion (reverts on false)
emit("event message")               // emit a chain event
return expression                    // return a value
```

## Built-in Functions

| Function | Args | Returns | Description |
|----------|------|---------|-------------|
| `caller` | 0 | `address` | Address of the transaction sender |
| `owner` | 0 | `address` | Address of the contract deployer |
| `epoch` | 0 | `u64` | Current chain epoch |
| `energy` | 0 | `u64` | Contract's remaining energy |
| `balance(addr)` | 1 | `u64` | On-chain token balance of an address |
| `transfer(to, amount)` | 2 | — | Transfer tokens to an address |
| `emit(msg)` | 1 | — | Emit a contract event |
| `require(cond, msg)` | 2 | — | Revert execution if condition is false |

## Lifecycle Hooks

Lifecycle hooks are called automatically by the EvaporChain runtime when contract energy state changes:

### `on_evaporate()`
Called when the contract's energy reaches zero and the contract is about to be evaporated. Use this for cleanup, final notifications, or archival.

### `on_grace()`
Called when the contract enters the grace period (energy depleted but not yet evaporated). The contract can still be saved by refreshing its energy.

### `on_refresh()`
Called when the contract receives an energy deposit, extending its lifetime.

## Example Contracts

### 1. Loyalty Points

A points system where a shop owner issues points to customers. Points can be spent. The entire loyalty program expires when contract energy runs out.

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

### 2. Expiring Event Ticket

A ticket that can only be used once. The ticket itself evaporates after the event window passes.

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

### 3. Decaying Auction

An auction where the entire auction evaporates if nobody bids in time. The winner is the highest bidder when the contract runs out of energy.

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
# Deploy an EvaporScript contract
curl -X POST http://localhost:3000/api/tx/deploy-script \
  -H "Content-Type: application/json" \
  -d '{
    "deployer": 1,
    "source_code": "contract Counter { state { count: u64 = 0 } fn increment(n: u64) { self.count += n } fn get() -> u64 { return self.count } on_evaporate() { emit(\"counter expired\") } }",
    "energy": 10000,
    "half_life": 200
  }'
```

### Call a method

```bash
# Call the increment method
curl -X POST http://localhost:3000/api/tx/call-script \
  -H "Content-Type: application/json" \
  -d '{
    "caller": 1,
    "contract_id": 1,
    "method": "increment",
    "args": "[{\"U64\": 42}]",
    "epoch": 10
  }'
```

## Execution Model

EvaporScript compiles to a stack-based bytecode (EvaporBytecode) that runs on the EvaporVM:

1. **Parse** — Source code is tokenized and parsed into an AST
2. **Compile** — The AST is compiled into bytecode with a method table and state schema
3. **Execute** — The VM executes bytecode against contract state with gas metering

### Gas Costs

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
