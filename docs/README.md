# EvaporChain — Getting Started

## What is EvaporChain?

EvaporChain is a blockchain where state expires by default. Every on-chain object has an energy budget that decays exponentially over time — unused state evaporates automatically, and only a cryptographic nullifier proof remains. Combined with recursive zero-knowledge proof folding, the chain gets *lighter* over time, not heavier.

## Connect to the Testnet

The public testnet API is available at:

```
http://<TESTNET_IP>:8080
```

- **Dashboard:** `http://<TESTNET_IP>:8080/`
- **Faucet:** `http://<TESTNET_IP>:8080/faucet`
- **API Base:** `http://<TESTNET_IP>:8080/api/`

## Get Testnet Tokens

Visit the faucet page or use curl:

```bash
curl -X POST http://<TESTNET_IP>:8080/api/faucet \
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
curl -X POST http://<TESTNET_IP>:8080/api/tx/transfer \
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
curl -X POST http://<TESTNET_IP>:8080/api/tx/create-object \
  -H "Content-Type: application/json" \
  -d '{"creator": 1, "object_id": 42, "energy": 5000, "half_life": 100}'
```

- `energy`: Initial energy budget (decays exponentially)
- `half_life`: Number of epochs for energy to halve

When energy reaches zero, the object enters a grace period, then evaporates.

## Deploy a Smart Contract

Template-based contracts:

```bash
curl -X POST http://<TESTNET_IP>:8080/api/tx/deploy-contract \
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

### Network diagnostics

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/network/peers` | Per-peer summary: `peer_id`, `ip`, `subnet`, `since_ms`, `score`, `age_seconds`, `infractions`, `last_seen_ms`. Lane R.15 added the last two — together they give a one-curl read of all key freeze-class signals (a peer with `score: -100, infractions: 0` got there by idle-decay; with `infractions: high` got there by misbehaviour). |
| GET | `/api/network/scores` | Diagnostic projection of the full Sybil score map, including ghost-entries (peers with a score but no live connection). `ghost_count > 0` is the freeze-class signal Lane R.* would have caught. Returns `{scores:[{peer_id, connected, ip, since_ms, score, infractions, last_seen_ms}], count, ghost_count}`. |

### Light-Cone DAG (Phase 4.4 antichain commit-cert digest)

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/light_cone` | Light-Cone DAG block count + "running alongside Tendermint" flag. |
| GET | `/api/light_cone/antichain_digest` | Phase 4.4 antichain commit-cert digest. Domain-separated 32-byte blake3 fingerprint of the closing antichain (`evaporchain-antichain-digest-v1`), validator-deterministic via sort-before-hash. Returns `{digest, closing_antichain, closing_antichain_size, running_alongside_tendermint}`. Operators `curl` across all cluster validators and pattern-match the digests; divergence is the freeze-class signal for antichain disagreement. Pairs with `mev_state_digest` (Phase 3.2) as the second canonical inter-validator digest. |

### Lambda-Fold Nova IVC

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/lambda_fold/nova` | Nova IVC accumulator state: `step_count`, `latest_epoch`, `is_identity`, etc. Active when governance flag `lambda_fold_mode = "nova"`. |
| GET | `/api/lambda_fold/nova/vk_bytes` | Compressed verifying-key bytes for the recursive SNARK. Light clients fetch this once + use it to verify chain proofs in essentially constant time regardless of fold count. |
| POST | `/api/lambda_fold/nova/verify` | Sublinear chain-proof verification — empirically locked at 23 ms @ 100 folds (1.083× of 23 ms @ 10 folds) on M4 release. |

### Decay-Lamport time

| Method | Endpoint / RPC | Description |
|--------|----------|-------------|
| RPC | `evap_getLamportClock()` | Energy-driven logical clock (Decay-Lamport Time, INVENTION_STACK §4.1 #3). Ticks once per `tick_quantum` units of chain-wide gas spent. Wired on both proposer-local AND gossip-follower commit paths so all validators advance in lockstep. |

### Frontier — Causal-CHSH cartel detector

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/api/cartel_alarm/run_gate` | Run the Causal-CHSH gate against operator-supplied chain trace data. Returns Pass/Fail/InputError + S statistic + per-bucket sample counts. Doctrine-locked thresholds (1.8/2.2/0.4) baked in. Lane O.5. |
| GET | `/api/cartel_alarm/chain_status` | The chain's own self-monitoring verdict from the rolling buffer (200 blocks, gate every 50 records). Includes `pending_events_count` (Lane O.8.2f) so dashboards see queue depth without draining. Lane O.8.1b. |
| GET | `/api/cartel_alarm/pending_events` | Drain the queue of `CartelAlarmEvent`s emitted when chain S crossed the doctrine ceiling AND governance set `cartel_alarm_mode = "alarm"`. Each event returned exactly once. Default `observe` mode keeps the queue empty. Lane O.8.2b. |

### Crooks-MEV refund (substrate-level)

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/mev/observations` | Recent sandwich-pattern detections from `evaporchain-mev-detect`. Includes `confidence_score_ppm` (parts-per-million; divide by 1_000_000 for f64). |
| POST | `/api/mev/dispute` | Operator dispute path — reject a false-positive observation. |

Refund settlement (`Transaction::Refund`) is governance-gated via `crooks_mev_settlement_mode ∈ {observe, enforce}` (default `observe`). Both `SimpleExecutor` and the parallel-executor serial phase wire the attacker-debit / victim-credit balance movement (Lanes Q.1 + S.1).

## Run Locally

```bash
git clone https://github.com/ss1738/EvaporChain.git
cd EvaporChain
cargo build --release
cargo run -p evaporchain-node -- --api --api-port 8080
```

Then open `http://localhost:8080` for the dashboard.
