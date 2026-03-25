# EvaporChain — Getting Started

## What is EvaporChain?

EvaporChain is a blockchain where state expires by default. Every on-chain object has an energy budget that decays exponentially over time — unused state evaporates automatically, and only a cryptographic nullifier proof remains. Combined with recursive zero-knowledge proof folding, the chain gets *lighter* over time, not heavier.

## Connect to the Testnet

The public testnet API is available at:

```
http://<TESTNET_IP>:3000
```

- **Dashboard:** `http://<TESTNET_IP>:3000/`
- **Faucet:** `http://<TESTNET_IP>:3000/faucet`
- **API Base:** `http://<TESTNET_IP>:3000/api/`

## Get Testnet Tokens

Visit the faucet page or use curl:

```bash
curl -X POST http://<TESTNET_IP>:3000/api/faucet \
  -H "Content-Type: application/json" \
  -d '{"address": 1}'
```

Response:
```json
{"success": true, "balance": 10000}
```

Each address can request tokens once per hour. You receive 10,000 EVAP per request.

## Send a Transfer

```bash
curl -X POST http://<TESTNET_IP>:3000/api/tx/transfer \
  -H "Content-Type: application/json" \
  -d '{"from": 1, "to": 2, "amount": 500, "nonce": 0}'
```

Response:
```json
{"success": true, "message": "Transfer queued: Account-1 -> Account-2 amount=500 (mempool=1)"}
```

## Create an Object with Energy

Objects are the core state primitive. They have energy that decays over time.

```bash
curl -X POST http://<TESTNET_IP>:3000/api/tx/create-object \
  -H "Content-Type: application/json" \
  -d '{"creator": 1, "object_id": 42, "energy": 5000, "half_life": 100}'
```

- `energy`: Initial energy budget (decays exponentially)
- `half_life`: Number of epochs for energy to halve

When energy reaches zero, the object enters a grace period, then evaporates.

## Deploy a Smart Contract

Template-based contracts:

```bash
curl -X POST http://<TESTNET_IP>:3000/api/tx/deploy-contract \
  -H "Content-Type: application/json" \
  -d '{
    "deployer": 1,
    "template": "DecayingToken",
    "init_args": "{\"name\":\"TestCoin\",\"symbol\":\"TC\",\"total_supply\":1000000,\"decay_half_life\":100,\"owner\":\"alice\"}",
    "energy": 10000,
    "half_life": 200
  }'
```

Available templates: `DecayingToken`, `MortalNFT`, `ThermodynamicEscrow`, `DecayingAuction`, `StakingPool`, `DAOVote`

## API Reference

### Status & Explorer

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/health` | Health check (returns `ok`) |
| GET | `/api/status` | Chain status: epoch, block height, object counts, uptime |
| GET | `/api/objects` | List all active state objects |
| GET | `/api/object/{id}` | Get a single object by hex ID |
| GET | `/api/accounts` | List all accounts with balances |
| GET | `/api/ghosts` | List all evaporated (ghost) objects |
| GET | `/api/blocks?limit=N` | Recent blocks (default limit: 50) |
| GET | `/api/block/{number}` | Get a specific block by number |
| GET | `/api/events?limit=N` | Recent chain events |
| GET | `/api/stats/timeline` | State size over time (active vs ghost counts) |
| GET | `/api/stats/summary` | Aggregate statistics |
| GET | `/api/network` | Network info (peer count, validator ID) |

### Contracts

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/contracts` | List all deployed contracts |
| GET | `/api/contract/{id}` | Get contract details by ID |
| POST | `/api/tx/deploy-contract` | Deploy a template contract |
| POST | `/api/tx/call-contract` | Call a method on a deployed contract |

### Transactions

| Method | Endpoint | Body | Description |
|--------|----------|------|-------------|
| POST | `/api/tx/transfer` | `{from, to, amount, nonce}` | Transfer tokens |
| POST | `/api/tx/create-object` | `{creator, object_id, energy, half_life}` | Create a state object |
| POST | `/api/tx/refresh` | `{object_id, energy_deposit}` | Refresh energy on an object |
| POST | `/api/tx/resurrect` | `{object_id, energy_deposit}` | Resurrect an evaporated object |

### Faucet

| Method | Endpoint | Body | Description |
|--------|----------|------|-------------|
| GET | `/faucet` | — | Faucet web page |
| POST | `/api/faucet` | `{address: u8}` | Request 10,000 testnet tokens |

## Run Locally

```bash
git clone https://github.com/ss1738/EvaporChain.git
cd EvaporChain
cargo build --release
cargo run -p evaporchain-node -- --api --api-port 3000
```

Then open `http://localhost:3000` for the dashboard.
