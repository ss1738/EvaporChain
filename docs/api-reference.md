# EvaporChain API Reference

Complete reference for every HTTP endpoint on the EvaporChain node. Base URL: `https://testnet.evaporchain.com`

All POST endpoints accept `Content-Type: application/json`.

---

## Chain

### GET /api/status

Current chain status.

```bash
curl https://testnet.evaporchain.com/api/status
```

Response:

```json
{
  "chain_name": "EvaporChain",
  "version": "0.2.0",
  "block_height": 1234,
  "epoch": 1234,
  "active_objects": 15,
  "ghost_count": 3,
  "total_evaporated": 3,
  "peer_count": 4,
  "state_root": "a1b2c3d4...",
  "proving_enabled": false,
  "uptime_seconds": 86400
}
```

### GET /health

Health check. Returns `200 ok`.

```bash
curl https://testnet.evaporchain.com/health
```

---

## Objects

### GET /api/objects

List all state objects sorted by state (Active > Risen > Grace > Ghost) then by energy descending.

```bash
curl https://testnet.evaporchain.com/api/objects
```

Response: array of objects.

```json
[
  {
    "id": "0a00000000000000000000000000000000000000000000000000000000000000",
    "name": "UserObj-10",
    "owner": "0100000000000000000000000000000000000000000000000000000000000000",
    "owner_name": "0x010000...0000",
    "energy": 10000,
    "max_energy": 10000,
    "half_life": 50,
    "state": "Active",
    "created_epoch": 0,
    "last_refreshed": 0,
    "grace_epoch": null,
    "current_energy": 7500,
    "decay_percentage": 25.0
  }
]
```

### GET /api/object/{id}

Get a single object by its 64-character hex ID.

```bash
curl https://testnet.evaporchain.com/api/object/0a00000000000000000000000000000000000000000000000000000000000000
```

Returns: single object (same schema as above). 404 if not found.

---

## Accounts

### GET /api/accounts

List all accounts sorted by balance descending.

```bash
curl https://testnet.evaporchain.com/api/accounts
```

```json
[
  {
    "address": "0100000000000000000000000000000000000000000000000000000000000000",
    "name": "0x010000...0000",
    "balance": 100000,
    "nonce": 5
  }
]
```

---

## Ghosts

### GET /api/ghosts

List all evaporated objects (ghost records).

```bash
curl https://testnet.evaporchain.com/api/ghosts
```

```json
[
  {
    "id": "0a00000000000000000000000000000000000000000000000000000000000000",
    "original_owner": "0100000000000000000000000000000000000000000000000000000000000000",
    "evaporated_epoch": 342,
    "data_hash": "b7c3f2a1..."
  }
]
```

---

## Blocks

### GET /api/blocks

List recent blocks (most recent first).

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `limit` | integer | 50 | Max blocks to return (max 500) |

```bash
curl "https://testnet.evaporchain.com/api/blocks?limit=5"
```

```json
[
  {
    "number": 1234,
    "epoch": 1234,
    "parent_hash": "abc123...",
    "state_root": "def456...",
    "tx_count": 3,
    "evaporations": 1,
    "entered_grace": 0,
    "timestamp": 1711324800,
    "active_objects": 15,
    "ghost_count": 3,
    "transactions": [
      { "type": "Transfer", "detail": "0x0100...→0x0200... amount=1000" }
    ]
  }
]
```

### GET /api/block/{number}

Get a specific block by height.

```bash
curl https://testnet.evaporchain.com/api/block/42
```

Returns: single block (same schema). 404 if not found.

---

## Events

### GET /api/events

Recent chain events.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `limit` | integer | 50 | Max events (max 200) |

```bash
curl "https://testnet.evaporchain.com/api/events?limit=10"
```

```json
{
  "events": [
    {
      "epoch": 342,
      "event_type": "evaporated",
      "message": "Object 0x0a evaporated (energy: 0)",
      "timestamp_ms": 1711324800000
    }
  ]
}
```

Event types: `created`, `grace`, `evaporated`, `refreshed`, `resurrected`, `transfer`.

---

## Statistics

### GET /api/stats/summary

Aggregate chain statistics.

```bash
curl https://testnet.evaporchain.com/api/stats/summary
```

```json
{
  "total_created": 25,
  "total_evaporated": 8,
  "total_resurrected": 2,
  "total_refreshed": 12,
  "avg_lifetime_epochs": 156.3,
  "total_transactions": 89
}
```

### GET /api/stats/timeline

Epoch-by-epoch state size timeline.

```bash
curl https://testnet.evaporchain.com/api/stats/timeline
```

```json
{
  "epochs": [
    { "epoch": 0, "active_count": 10, "ghost_count": 0, "total_energy": 100000 },
    { "epoch": 1, "active_count": 10, "ghost_count": 0, "total_energy": 99500 }
  ]
}
```

---

## Network

### GET /api/network

Network peer information.

```bash
curl https://testnet.evaporchain.com/api/network
```

```json
{ "peer_count": 4 }
```

---

## Transactions

### POST /api/tx/transfer

Transfer EVAP tokens between accounts.

```bash
curl -X POST https://testnet.evaporchain.com/api/tx/transfer \
  -H "Content-Type: application/json" \
  -d '{"from": 1, "to": 2, "amount": 5000, "nonce": 0}'
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `from` | integer | yes | Sender address byte (0-255) |
| `to` | integer | yes | Recipient address byte (0-255) |
| `amount` | integer | yes | Amount to transfer |
| `nonce` | integer | yes | Sender nonce |

```json
{ "success": true, "message": "Transfer queued: 0x010000...→0x020000... amount=5000 (mempool=1)" }
```

### POST /api/tx/create-object

Create a new decaying state object.

```bash
curl -X POST https://testnet.evaporchain.com/api/tx/create-object \
  -H "Content-Type: application/json" \
  -d '{"creator": 1, "object_id": 42, "energy": 10000, "half_life": 50}'
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `creator` | integer | yes | Creator address byte (0-255) |
| `object_id` | integer | yes | Object ID byte (1-255) |
| `energy` | integer | yes | Initial energy budget |
| `half_life` | integer | yes | Epochs for energy to halve |

### POST /api/tx/refresh

Deposit energy into an existing object.

```bash
curl -X POST https://testnet.evaporchain.com/api/tx/refresh \
  -H "Content-Type: application/json" \
  -d '{"object_id": 42, "energy_deposit": 5000}'
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `object_id` | integer | yes | Object ID byte |
| `energy_deposit` | integer | yes | Energy to add |

### POST /api/tx/resurrect

Resurrect an evaporated ghost object.

```bash
curl -X POST https://testnet.evaporchain.com/api/tx/resurrect \
  -H "Content-Type: application/json" \
  -d '{"object_id": 42, "energy_deposit": 10000}'
```

Same fields as refresh.

---

## Contracts

### GET /api/contracts

List all deployed contracts (both template and EvaporScript).

```bash
curl https://testnet.evaporchain.com/api/contracts
```

```json
{
  "contracts": [
    {
      "id": 1,
      "template": "DecayingToken",
      "creator": "0x010000...0000",
      "energy": 50000,
      "half_life": 500,
      "created_epoch": 10,
      "evaporated": false
    }
  ]
}
```

### GET /api/contract/{id}

Get a single contract by ID, including its state.

```bash
curl https://testnet.evaporchain.com/api/contract/1
```

### POST /api/tx/deploy-contract

Deploy a template contract.

```bash
curl -X POST https://testnet.evaporchain.com/api/tx/deploy-contract \
  -H "Content-Type: application/json" \
  -d '{
    "deployer": 1,
    "template": "DecayingToken",
    "init_args": {"name": "MyToken", "symbol": "MTK", "supply": 1000000},
    "energy": 50000,
    "half_life": 500
  }'
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `deployer` | integer | yes | Deployer address byte |
| `template` | string | yes | Template name |
| `init_args` | object | yes | Initialization arguments |
| `energy` | integer | yes | Initial energy |
| `half_life` | integer | yes | Epochs for energy to halve |
| `rules` | object | no | Optional contract rules |

Templates: `DecayingToken`, `MortalNFT`, `ThermodynamicEscrow`, `DecayingAuction`, `StakingPool`, `DAOVote`.

### POST /api/tx/call-contract

Call a method on a deployed contract.

```bash
curl -X POST https://testnet.evaporchain.com/api/tx/call-contract \
  -H "Content-Type: application/json" \
  -d '{
    "caller": 1,
    "contract_id": 1,
    "method": "transfer",
    "args": {"to": 2, "amount": 100},
    "epoch": 42
  }'
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `caller` | integer | yes | Caller address byte |
| `contract_id` | integer | yes | Contract ID |
| `method` | string | yes | Method name |
| `args` | object | yes | Method arguments |
| `epoch` | integer | yes | Current epoch |

---

## Faucet

### POST /api/faucet

Claim testnet EVAP tokens. Rate limited to once per address per hour. Grants 10,000 EVAP.

```bash
curl -X POST https://testnet.evaporchain.com/api/faucet \
  -H "Content-Type: application/json" \
  -d '{"address": 42}'
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `address` | integer | yes | Address byte (0-255) |

```json
{ "success": true, "balance": 10000 }
```

---

## NFTs

### GET /api/nfts

List all mortal NFTs with current energy levels.

```bash
curl https://testnet.evaporchain.com/api/nfts
```

```json
[
  {
    "id": 1,
    "name": "Eternal Flame",
    "collection": "Genesis",
    "owner": "0x7f0000...0000",
    "metadata_hash": "a1b2c3...",
    "energy": 10000,
    "max_energy": 10000,
    "half_life": 500,
    "minted_epoch": 0,
    "last_refreshed": 0,
    "state": "Active",
    "current_energy": 9800,
    "decay_pct": 2.0,
    "epochs_remaining": 4482
  }
]
```

### GET /api/nft/{id}

Get a single NFT by ID.

```bash
curl https://testnet.evaporchain.com/api/nft/1
```

### POST /api/nft/mint

Mint a new mortal NFT.

```bash
curl -X POST https://testnet.evaporchain.com/api/nft/mint \
  -H "Content-Type: application/json" \
  -d '{
    "name": "My Art",
    "collection": "Custom",
    "owner": "0x910000...0000",
    "energy": 5000,
    "half_life": 100
  }'
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | yes | NFT name |
| `collection` | string | yes | Collection name |
| `owner` | string | yes | Owner address |
| `energy` | integer | yes | Initial energy |
| `half_life` | integer | yes | Decay half-life in epochs |

### POST /api/nft/transfer

Transfer an NFT to a new owner.

```bash
curl -X POST https://testnet.evaporchain.com/api/nft/transfer \
  -H "Content-Type: application/json" \
  -d '{"id": 1, "to": "0x2b0000...0000"}'
```

### POST /api/nft/refresh

Refresh an NFT's energy (or resurrect a ghost NFT).

```bash
curl -X POST https://testnet.evaporchain.com/api/nft/refresh \
  -H "Content-Type: application/json" \
  -d '{"id": 1, "energy": 5000}'
```

---

## Tokens

### GET /api/tokens

List all deployed decaying tokens.

```bash
curl https://testnet.evaporchain.com/api/tokens
```

```json
[
  {
    "id": 1,
    "name": "EvaporChain",
    "symbol": "EVAP",
    "total_supply": 1000000,
    "current_supply": 999000,
    "decay_half_life": 1000,
    "deployed_epoch": 0,
    "deployer": "0x7f0000...0000",
    "decay_percentage": 0.1,
    "holder_count": 5,
    "holders": [
      { "address": "0x7f0000...0000", "balance": 499500 }
    ]
  }
]
```

### GET /api/token/{id}

Get a single token by ID.

```bash
curl https://testnet.evaporchain.com/api/token/1
```

### POST /api/token/deploy

Deploy a new decaying token.

```bash
curl -X POST https://testnet.evaporchain.com/api/token/deploy \
  -H "Content-Type: application/json" \
  -d '{
    "name": "MyToken",
    "symbol": "MTK",
    "total_supply": 500000,
    "decay_half_life": 200,
    "deployer": "0x910000...0000",
    "initial_holders": {
      "0x910000...0000": 300000,
      "0x2b0000...0000": 200000
    }
  }'
```

### POST /api/token/transfer

Transfer tokens between addresses.

```bash
curl -X POST https://testnet.evaporchain.com/api/token/transfer \
  -H "Content-Type: application/json" \
  -d '{"token_id": 1, "from": "0x7f0000...0000", "to": "0x910000...0000", "amount": 1000}'
```

### POST /api/token/balance

Check a specific address's balance for a token.

```bash
curl -X POST https://testnet.evaporchain.com/api/token/balance \
  -H "Content-Type: application/json" \
  -d '{"token_id": 1, "address": "0x910000...0000"}'
```

---

## Staking

### GET /api/staking/pools

List all staking pools with staker details.

```bash
curl https://testnet.evaporchain.com/api/staking/pools
```

```json
[
  {
    "id": 1,
    "name": "Genesis Validator Pool",
    "reward_rate": 100,
    "reward_decay_hl": 50,
    "total_staked": 95000,
    "created_epoch": 0,
    "staker_count": 3,
    "stakers": [
      {
        "address": "0x910000...0000",
        "amount": 50000,
        "staked_epoch": 0,
        "pending_rewards": 520,
        "last_claim_epoch": 0,
        "total_claimed": 0,
        "total_decayed": 0,
        "reward_decay_pct": 8.5
      }
    ]
  }
]
```

### GET /api/staking/pool/{id}

Get a single pool by ID.

```bash
curl https://testnet.evaporchain.com/api/staking/pool/1
```

### POST /api/staking/stake

Stake EVAP in a pool.

```bash
curl -X POST https://testnet.evaporchain.com/api/staking/stake \
  -H "Content-Type: application/json" \
  -d '{"pool_id": 1, "address": "0x910000...0000", "amount": 10000}'
```

### POST /api/staking/unstake

Unstake EVAP from a pool.

```bash
curl -X POST https://testnet.evaporchain.com/api/staking/unstake \
  -H "Content-Type: application/json" \
  -d '{"pool_id": 1, "address": "0x910000...0000", "amount": 5000}'
```

### POST /api/staking/claim

Claim pending staking rewards. Rewards decay over time — claim before they evaporate.

```bash
curl -X POST https://testnet.evaporchain.com/api/staking/claim \
  -H "Content-Type: application/json" \
  -d '{"pool_id": 1, "address": "0x910000...0000"}'
```

---

## DAO

### GET /api/dao/proposals

List all governance proposals.

```bash
curl https://testnet.evaporchain.com/api/dao/proposals
```

```json
[
  {
    "id": 1,
    "title": "Increase base reward rate to 150 EVAP/epoch",
    "description": "The current reward rate is insufficient...",
    "options": ["For", "Against", "Abstain"],
    "created_epoch": 0,
    "voting_period": 200,
    "end_epoch": 200,
    "creator": "0x910000...0000",
    "status": "Active",
    "total_votes": 95000,
    "vote_totals": { "For": 80000, "Against": 15000, "Abstain": 0 },
    "epochs_remaining": 150,
    "evaporated_epoch": null,
    "voter_count": 3
  }
]
```

Status values: `Active`, `Passed:For`, `Passed:Against`, `Evaporated`.

### GET /api/dao/proposal/{id}

Get a single proposal by ID.

```bash
curl https://testnet.evaporchain.com/api/dao/proposal/1
```

### POST /api/dao/propose

Create a new governance proposal.

```bash
curl -X POST https://testnet.evaporchain.com/api/dao/propose \
  -H "Content-Type: application/json" \
  -d '{
    "title": "Fund developer grants",
    "description": "Allocate 50,000 EVAP for ecosystem development",
    "options": ["For", "Against", "Abstain"],
    "voting_period": 100,
    "creator": "0x910000...0000"
  }'
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `title` | string | yes | Proposal title |
| `description` | string | yes | Detailed description |
| `options` | array | yes | Voting options |
| `voting_period` | integer | yes | Duration in epochs |
| `creator` | string | yes | Creator address |

### POST /api/dao/vote

Vote on an active proposal.

```bash
curl -X POST https://testnet.evaporchain.com/api/dao/vote \
  -H "Content-Type: application/json" \
  -d '{
    "proposal_id": 1,
    "voter": "0x910000...0000",
    "option": "For",
    "weight": 10000
  }'
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `proposal_id` | integer | yes | Proposal ID |
| `voter` | string | yes | Voter address |
| `option` | string | yes | Vote option (must match proposal options) |
| `weight` | integer | yes | Vote weight |

---

## Dashboard Pages

| Route | Description |
|-------|-------------|
| `/` | Wallet (default) |
| `/wallet` | Wallet dashboard |
| `/explorer` | Block explorer |
| `/nft` | NFT marketplace |
| `/tokens` | Token management |
| `/staking` | Staking dashboard |
| `/dao` | Governance proposals |
| `/faucet` | Testnet faucet |

---

## Error Responses

All endpoints return standard HTTP status codes:

| Code | Meaning |
|------|---------|
| 200 | Success |
| 400 | Bad request (invalid parameters) |
| 404 | Resource not found |
| 429 | Rate limited (faucet) |
| 500 | Internal server error |

Error responses include a JSON body:

```json
{ "error": "description of what went wrong" }
```

## CORS

All API endpoints support CORS (Cross-Origin Resource Sharing) with `Access-Control-Allow-Origin: *`.
